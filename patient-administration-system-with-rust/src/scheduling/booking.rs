//! Part of [`super`]: booking.

use super::*;

impl SchedulingService {
    /// Book `slot_id` for `person_id`. Fails (changing nothing) if the slot is not free or the
    /// patient already has an overlapping appointment.
    pub(crate) async fn book_tx(
        txn: &DatabaseTransaction,
        slot_id: Uuid,
        person_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        Self::book_inner(txn, slot_id, person_id, EntryLink::None, ctx).await
    }

    /// A room, clinician or piece of equipment cannot be in two places at once (CORE-3): refuse a booking
    /// when a *different* schedule with the same owner already holds a busy slot overlapping this one. The
    /// owner is locked for the transaction, so two bookings arriving together through two schedules cannot
    /// both pass the check. (Overlapping slots within one schedule are that schedule's own business: a
    /// schedule may deliberately offer several places at one time.)
    pub(super) async fn check_owner_free(
        txn: &DatabaseTransaction,
        slot: &crate::models::schedule::Slot,
    ) -> Result<()> {
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock(hashtextextended(owner_kind || ':' || owner_id::text, 0)) \
             FROM schedules WHERE id = $1",
            [slot.schedule_id.into()],
        ))
        .await
        .map_err(Error::Database)?;
        let clash = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM slots s2 \
                 JOIN schedules sc2 ON sc2.id = s2.schedule_id \
                 JOIN schedules sc1 ON sc1.id = $1 \
                 WHERE sc2.owner_kind = sc1.owner_kind AND sc2.owner_id = sc1.owner_id \
                   AND sc2.id <> sc1.id \
                   AND (s2.status = 'busy' OR EXISTS (SELECT 1 FROM appointments a WHERE a.slot_id = s2.id \
                        AND a.deleted_at IS NULL AND a.status NOT IN ('cancelled','no_show'))) \
                   AND s2.start_datetime < $3 AND s2.end_datetime > $2 LIMIT 1",
                [
                    slot.schedule_id.into(),
                    slot.start_datetime.into(),
                    slot.end_datetime.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        if clash.is_some() {
            return Err(Error::conflict(
                "that room, clinician or equipment is already booked at that time",
            ));
        }
        Ok(())
    }

    pub(crate) async fn book_inner(
        txn: &DatabaseTransaction,
        slot_id: Uuid,
        person_id: Uuid,
        link: EntryLink,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        if let EntryLink::Explicit(entry_id) = link {
            let entry = WaitlistRepository::find_entry_by_id(txn, entry_id)
                .await?
                .ok_or_else(|| Error::not_found(format!("waitlist entry {entry_id}")))?;
            if entry.person_id != person_id {
                return Err(Error::conflict(
                    "that waiting-list entry belongs to a different patient",
                ));
            }
            if entry.status != WaitlistStatus::Waiting {
                return Err(Error::conflict(format!(
                    "waiting-list entry {entry_id} is {:?}, not waiting",
                    entry.status
                )));
            }
        }
        let slot = SlotRepository::select_for_update(txn, slot_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
        if slot.status != SlotStatus::Free {
            return Err(Error::conflict(format!(
                "slot {slot_id} is not free (status={:?})",
                slot.status
            )));
        }
        // A group session (NEW-18) takes bookings until its declared capacity; the row lock above means two
        // bookings arriving together are counted one after the other.
        let (capacity, booked) = crate::group_sessions::places(txn, slot_id).await?;
        if booked >= capacity {
            return Err(Error::conflict(format!("group session {slot_id} is full")));
        }
        Self::check_owner_free(txn, &slot).await?;
        let overlapping = AppointmentRepository::find_overlapping_for_patient(
            txn,
            person_id,
            slot.start_datetime,
            slot.end_datetime,
        )
        .await?;
        if !overlapping.is_empty() {
            return Err(Error::conflict(
                "patient already has an overlapping appointment",
            ));
        }
        // A one-to-one slot is busy once booked; a group session only once its last place is taken.
        if booked + 1 >= capacity {
            SlotRepository::update_status(txn, slot_id, SlotStatus::Busy).await?;
        }
        let mut appt = Appointment::new(person_id, slot.start_datetime, slot.end_datetime);
        appt.slot_id = Some(slot.id);
        if let EntryLink::Explicit(e) | EntryLink::Inherit(e) = link {
            appt.from_waitlist_entry_id = Some(e);
        }
        appt.status = appt.status.try_transition_to(AppointmentStatus::Booked)?;
        let appt = AppointmentRepository::create(txn, &appt).await?;
        Self::sync_waitlist(txn, &appt, WaitlistStatus::Booked, ctx).await?;
        crate::standby::mark_refilled(txn, slot.id, &appt).await?;
        // A patient who needs an interpreter gets one requested with every booking (NEW-5).
        crate::interpreter::on_booked(txn, &appt).await?;
        AuditLogRepository::log(
            txn,
            "appointment",
            appt.id,
            "book",
            None,
            Some(serde_json::to_value(&appt).unwrap_or_default()),
            ctx,
        )
        .await?;
        OutboxRepository::publish(
            txn,
            "AppointmentBooked",
            &serde_json::json!({
                "appointment_id": appt.id,
                "person_id": person_id,
                "slot_id": slot.id,
                "start": appt.start_datetime,
            }),
        )
        .await?;
        Ok(appt)
    }

    /// Book a slot for the patient on a waiting-list entry and mark the entry `booked` (CORE-2).
    pub async fn book_for_entry(
        &self,
        entry_id: Uuid,
        slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let entry = WaitlistRepository::find_entry_by_id(txn, entry_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("waitlist entry {entry_id}")))?;
                    Self::book_inner(
                        txn,
                        slot_id,
                        entry.person_id,
                        EntryLink::Explicit(entry_id),
                        &ctx,
                    )
                    .await
                })
            })
            .await
            .map_err(Error::from)?;
        self.publish_booked(&appt).await;
        Ok(appt)
    }

    pub async fn book_slot(
        &self,
        slot_id: Uuid,
        person_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move { Self::book_tx(txn, slot_id, person_id, &ctx).await })
            })
            .await
            .map_err(Error::from)?;
        self.publish_booked(&appt).await;
        Ok(appt)
    }
}

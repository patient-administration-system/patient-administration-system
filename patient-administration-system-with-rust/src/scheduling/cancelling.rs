//! Part of [`super`]: cancelling.

use super::*;
use crate::db::sql::Transact;

impl SchedulingService {
    /// Cancel an appointment, free its slot, and record the facts of CAP-1.
    pub(crate) async fn cancel_tx(
        txn: &crate::db::sql::Txn,
        appointment_id: Uuid,
        details: CancelDetails,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let mut appt = AppointmentRepository::find_by_id(txn, appointment_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
        appt.status = appt
            .status
            .try_transition_to(AppointmentStatus::Cancelled)?;
        let now = chrono::Utc::now();
        appt.cancellation_reason = Some(
            details
                .legacy_reason
                .unwrap_or_else(|| details.cancelled_by.legacy_reason()),
        );
        appt.cancelled_at = Some(now);
        appt.cancelled_by = Some(details.cancelled_by);
        appt.barrier_code = details.barrier_code;
        appt.notice_days = Some(notice_days(now, appt.start_datetime));
        appt.updated_at = now;
        let appt = AppointmentRepository::update(txn, &appt).await?;
        if let Some(sid) = appt.slot_id {
            SlotRepository::update_status(txn, sid, SlotStatus::Free).await?;
        }
        // A cancelled booking puts the patient back on the waiting list. It never removes them.
        Self::sync_waitlist(txn, &appt, WaitlistStatus::Waiting, ctx).await?;
        if let Some(class) = appt.release_class {
            crate::standby::record_release(txn, &appt, class).await?;
        }
        crate::interpreter::on_cancelled(txn, appt.id).await?;
        // Why, as a code a report can group by: the barrier the patient gave, else who ended it (CAP-6).
        let reason = match (details.legacy_reason, details.barrier_code) {
            (Some(CancellationReason::Rescheduled), _) => "rescheduled".to_string(),
            (_, Some(b)) => b.as_str().to_string(),
            _ => format!("{}_cancelled", enum_name(&details.cancelled_by)),
        };
        AuditLogRepository::log_with(
            txn,
            "appointment",
            appt.id,
            "cancel",
            None,
            Some(serde_json::to_value(&appt).unwrap_or_default()),
            Some(&reason),
            Some(serde_json::json!({
                "cancelled_by": details.cancelled_by,
                "barrier_code": details.barrier_code,
                "notice_days": appt.notice_days,
                "release_class": appt.release_class,
            })),
            ctx,
        )
        .await?;
        OutboxRepository::publish(
            txn,
            "AppointmentCancelled",
            &serde_json::json!({
                "appointment_id": appt.id,
                "slot_id": appt.slot_id,
                "start_datetime": appt.start_datetime,
                "cancelled_by": appt.cancelled_by,
                "barrier_code": appt.barrier_code,
                "notice_days": appt.notice_days,
                "release_class": appt.release_class,
            }),
        )
        .await?;
        Ok(appt)
    }

    /// Move a patient from `appointment_id` to `new_slot_id`: everything is checked first, then
    /// the old appointment is released as `rescheduled` and the new one booked, so a refusal
    /// leaves the patient exactly where they were. Returns `(old, new)`.
    pub(crate) async fn rebook_tx(
        txn: &crate::db::sql::Txn,
        appointment_id: Uuid,
        new_slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<(Appointment, Appointment)> {
        let old = AppointmentRepository::find_by_id(txn, appointment_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
        if !matches!(
            old.status,
            AppointmentStatus::Booked | AppointmentStatus::Proposed
        ) {
            return Err(Error::invalid_transition(format!(
                "appointment {appointment_id} is {:?}; only a booked appointment can be moved",
                old.status
            )));
        }
        if old.slot_id == Some(new_slot_id) {
            return Err(Error::conflict(
                "that is the slot the appointment is already in",
            ));
        }
        let slot = SlotRepository::select_for_update(txn, new_slot_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("slot {new_slot_id}")))?;
        if slot.status != SlotStatus::Free {
            return Err(Error::conflict(format!(
                "slot {new_slot_id} is no longer free"
            )));
        }
        let clashes = AppointmentRepository::find_overlapping_for_patient_excluding(
            txn,
            old.person_id,
            slot.start_datetime,
            slot.end_datetime,
            old.id,
        )
        .await?;
        if !clashes.is_empty() {
            return Err(Error::conflict(
                "patient already has an overlapping appointment",
            ));
        }
        let cancelled = Self::cancel_tx(
            txn,
            appointment_id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: None,
                legacy_reason: Some(CancellationReason::Rescheduled),
            },
            ctx,
        )
        .await?;
        let link = old
            .from_waitlist_entry_id
            .map_or(EntryLink::None, EntryLink::Inherit);
        let booked = Self::book_inner(txn, new_slot_id, old.person_id, link, ctx).await?;
        Ok((cancelled, booked))
    }

    /// Keep the waiting-list entry an appointment came from in step with it (CORE-2). It moves an
    /// entry only when this appointment is responsible for the entry's current state, so a
    /// removed or already-treated entry is never disturbed. A miss or cancellation returns the
    /// patient to `waiting`; it never removes them (CAP-7).
    pub(crate) async fn sync_waitlist(
        txn: &crate::db::sql::Txn,
        appt: &Appointment,
        to: WaitlistStatus,
        ctx: &UserContext,
    ) -> Result<()> {
        let Some(entry_id) = appt.from_waitlist_entry_id else {
            return Ok(());
        };
        let Some(mut entry) = WaitlistRepository::find_entry_by_id(txn, entry_id).await? else {
            return Ok(());
        };
        let responsible = match to {
            WaitlistStatus::Booked => entry.status == WaitlistStatus::Waiting,
            WaitlistStatus::Waiting | WaitlistStatus::Treated => {
                entry.status == WaitlistStatus::Booked
            }
            WaitlistStatus::Removed => false,
        };
        if !responsible {
            return Ok(());
        }
        let from = entry.status;
        entry.status = to;
        entry.updated_at = chrono::Utc::now();
        let entry = WaitlistRepository::update_entry(txn, &entry).await?;
        AuditLogRepository::log(
            txn,
            "waitlist_entry",
            entry.id,
            "status_changed",
            Some(serde_json::json!({ "status": from })),
            Some(serde_json::json!({ "status": to, "appointment_id": appt.id })),
            ctx,
        )
        .await?;
        OutboxRepository::publish(
            txn,
            "WaitlistStatusChanged",
            &serde_json::json!({
                "entry_id": entry.id, "from": from, "to": to, "appointment_id": appt.id,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn cancel(
        &self,
        appointment_id: Uuid,
        reason: CancellationReason,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let cancelled_by = match reason {
            CancellationReason::ProviderRequest => CancelledBy::Provider,
            CancellationReason::PatientRequest | CancellationReason::Rescheduled => {
                CancelledBy::Patient
            }
            CancellationReason::NoShow | CancellationReason::Other => CancelledBy::System,
        };
        self.cancel_with(
            appointment_id,
            CancelDetails {
                cancelled_by,
                barrier_code: None,
                legacy_reason: Some(reason),
            },
            ctx,
        )
        .await
    }

    /// Cancel and record who cancelled, the patient's barrier if they gave one, when, and how many
    /// whole days of notice the service had (CAP-1). The outbox event carries the same facts so a
    /// consumer can tell an early release from a late one without another query.
    pub async fn cancel_with(
        &self,
        appointment_id: Uuid,
        details: CancelDetails,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move { Self::cancel_tx(txn, appointment_id, details, &ctx).await })
            })
            .await?;
        self.publish_cancelled(&appt).await;
        Ok(appt)
    }

    /// [`Self::cancel_with`] that refuses, with `PreconditionFailed`, when the appointment has changed
    /// since the caller read it (OFH-1). `expected` is the `updated_at` the caller saw, compared to the
    /// microsecond (the precision the database keeps). `None` skips the comparison. The row is locked
    /// first, so two requests that read the same version cannot both pass: the second waits, then gets `412`.
    pub async fn cancel_if_unchanged(
        &self,
        appointment_id: Uuid,
        details: CancelDetails,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    AppointmentRepository::lock_if_current(txn, appointment_id, expected).await?;
                    Self::cancel_tx(txn, appointment_id, details, &ctx).await
                })
            })
            .await?;
        self.publish_cancelled(&appt).await;
        Ok(appt)
    }

    /// Move an appointment to another free slot, atomically. See `Self::rebook_tx`.
    pub async fn rebook(
        &self,
        appointment_id: Uuid,
        new_slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<(Appointment, Appointment)> {
        let ctx = ctx.clone();
        let (old, new) = self
            .db
            .in_txn::<_, (Appointment, Appointment)>(|txn| {
                Box::pin(
                    async move { Self::rebook_tx(txn, appointment_id, new_slot_id, &ctx).await },
                )
            })
            .await?;
        self.publish_cancelled(&old).await;
        self.publish_booked(&new).await;
        Ok((old, new))
    }
}

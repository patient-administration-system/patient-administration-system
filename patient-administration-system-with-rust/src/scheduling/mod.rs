//! scheduling — Slot/Appointment booking service + recurring series.

pub mod series;

pub use series::{
    CreateSeriesInput, CreateSeriesResult, PreviewResult, SeriesService, SeriesWithOccurrences,
};

use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::waitlist::WaitlistRepository;
use crate::db::repositories::{
    appointment::AppointmentRepository,
    audit::{AuditLogRepository, UserContext},
    outbox::OutboxRepository,
    slot::SlotRepository,
};
use crate::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancellationReason, CancelledBy, notice_days,
};
use crate::models::schedule::SlotStatus;
use crate::models::waitlist::WaitlistStatus;
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

/// How a new booking relates to a waiting-list entry (CORE-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryLink {
    /// Not from a waiting list.
    None,
    /// Booking *from* this entry: it must exist, belong to the patient and be waiting.
    Explicit(Uuid),
    /// A rebook carrying the old appointment's entry forward. Lenient: if the entry has moved on
    /// (removed, treated) the new appointment is simply not linked to a live entry.
    Inherit(Uuid),
}

/// What the caller knows about a cancellation (CAP-1).
#[derive(Debug, Clone, Copy)]
pub struct CancelDetails {
    pub cancelled_by: CancelledBy,
    pub barrier_code: Option<BarrierCode>,
    /// Overrides the reason derived from `cancelled_by` (used by `rescheduled`).
    pub legacy_reason: Option<CancellationReason>,
}

pub struct SchedulingService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
    /// The transport service (CAP-15), set after construction because it needs the person client. Shared
    /// by every clone of the `Arc<SchedulingService>`, so swapping it reaches all of them.
    transport: std::sync::RwLock<Option<Arc<crate::transport::TransportService>>>,
    /// The no-show probability (CAP-12), set the same way. Only ever added support, and off unless approved.
    noshow: std::sync::RwLock<Option<Arc<crate::noshow::NoShowService>>>,
    /// The backfill offers (PILOT-8), set the same way.
    backfill: std::sync::RwLock<Option<Arc<crate::backfill::BackfillService>>>,
}

impl SchedulingService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            db,
            publisher,
            transport: std::sync::RwLock::new(None),
            backfill: std::sync::RwLock::new(None),
            noshow: std::sync::RwLock::new(None),
        }
    }

    pub fn set_transport(&self, t: Arc<crate::transport::TransportService>) {
        *self
            .transport
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(t);
    }

    /// A clone out of the lock, so the guard is never held across an await.
    fn noshow(&self) -> Option<Arc<crate::noshow::NoShowService>> {
        self.noshow
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_noshow(&self, n: Arc<crate::noshow::NoShowService>) {
        *self
            .noshow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(n);
    }

    pub fn set_backfill(&self, b: Arc<crate::backfill::BackfillService>) {
        *self
            .backfill
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(b);
    }

    fn backfill(&self) -> Option<Arc<crate::backfill::BackfillService>> {
        self.backfill
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn transport(&self) -> Option<Arc<crate::transport::TransportService>> {
        self.transport
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// After an appointment has been confirmed: request a ride if the patient is eligible and has
    /// consented (CAP-15). Best effort: the confirmation stands whatever happens.
    pub(crate) async fn after_confirmed(&self, appointment_id: Uuid) {
        if let Some(t) = self.transport()
            && let Err(e) = t.on_confirmed(appointment_id).await
        {
            tracing::warn!(target: "pas::transport", "transport request failed: {e}");
        }
    }

    // --- transaction-level building blocks -------------------------------------------------
    //
    // These run inside a caller-supplied transaction so that larger operations (rebooking, a
    // patient's reply arriving from outside) can combine them atomically. They publish nothing;
    // the caller publishes the returned events after the commit.

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
    async fn check_owner_free(
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
                   AND sc2.id <> sc1.id AND s2.status = 'busy' \
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
        SlotRepository::update_status(txn, slot_id, SlotStatus::Busy).await?;
        let mut appt = Appointment::new(person_id, slot.start_datetime, slot.end_datetime);
        appt.slot_id = Some(slot.id);
        if let EntryLink::Explicit(e) | EntryLink::Inherit(e) = link {
            appt.from_waitlist_entry_id = Some(e);
        }
        appt.status = appt.status.try_transition_to(AppointmentStatus::Booked)?;
        let appt = AppointmentRepository::create(txn, &appt).await?;
        Self::sync_waitlist(txn, &appt, WaitlistStatus::Booked, ctx).await?;
        crate::standby::mark_refilled(txn, slot.id, &appt).await?;
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

    /// Cancel an appointment, free its slot, and record the facts of CAP-1.
    pub(crate) async fn cancel_tx(
        txn: &DatabaseTransaction,
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
        txn: &DatabaseTransaction,
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
        txn: &DatabaseTransaction,
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

    // --- public operations ----------------------------------------------------------------

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
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move { Self::cancel_tx(txn, appointment_id, details, &ctx).await })
            })
            .await
            .map_err(Error::from)?;
        self.publish_cancelled(&appt).await;
        Ok(appt)
    }

    /// Move an appointment to another free slot, atomically. See [`Self::rebook_tx`].
    pub async fn rebook(
        &self,
        appointment_id: Uuid,
        new_slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<(Appointment, Appointment)> {
        let ctx = ctx.clone();
        let (old, new) = self
            .db
            .transaction::<_, (Appointment, Appointment), Error>(|txn| {
                Box::pin(
                    async move { Self::rebook_tx(txn, appointment_id, new_slot_id, &ctx).await },
                )
            })
            .await
            .map_err(Error::from)?;
        self.publish_cancelled(&old).await;
        self.publish_booked(&new).await;
        Ok((old, new))
    }

    /// Publish a domain event on this service's publisher (best effort, after a commit).
    pub(crate) async fn publish_event(&self, event: DomainEvent) -> Result<()> {
        self.publisher.publish(event).await
    }

    pub(crate) async fn publish_booked(&self, appt: &Appointment) {
        // A new booking gets its no-show probability, if the score is switched on and approved (CAP-12). It
        // only adds a task for a person; the booking is already made and nothing here can change it.
        if crate::noshow::NoShowService::switched_on()
            && let Some(n) = self.noshow()
        {
            let _ = n.assess(appt.id).await;
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentBooked",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await;
    }

    pub(crate) async fn publish_cancelled(&self, appt: &Appointment) {
        // The freed slot is offered to the next patient on standby, when offers are switched on (PILOT-8).
        if let Some(b) = self.backfill()
            && let Err(e) = b.on_cancelled(appt.id).await
        {
            tracing::warn!(target: "pas::backfill", "offer failed: {e}");
        }
        // A cancelled (or moved) appointment no longer needs its ride (CAP-15).
        if let Some(t) = self.transport()
            && let Err(e) = t.on_cancelled(appt.id).await
        {
            tracing::warn!(target: "pas::transport", "transport cancel failed: {e}");
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentCancelled",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await;
    }

    /// Record the barrier for an appointment that has already ended in a cancellation or a miss,
    /// for example after a welfare call. Refused for an appointment that is still to happen.
    pub async fn record_barrier(
        &self,
        appointment_id: Uuid,
        barrier: BarrierCode,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        self.db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut appt = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    if !matches!(appt.status, AppointmentStatus::Cancelled | AppointmentStatus::NoShow) {
                        return Err(Error::invalid_transition(
                            "a barrier can only be recorded for a cancelled appointment or a no-show",
                        ));
                    }
                    let before = appt.barrier_code;
                    appt.barrier_code = Some(barrier);
                    appt.updated_at = chrono::Utc::now();
                    let appt = AppointmentRepository::update(txn, &appt).await?;
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        appt.id,
                        "record_barrier",
                        Some(serde_json::json!({ "barrier_code": before })),
                        Some(serde_json::json!({ "barrier_code": appt.barrier_code })),
                        &ctx_clone,
                    )
                    .await?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)
    }

    /// Where the patient is now (CORE-4). Only for someone who has arrived and not yet been completed;
    /// an empty room clears it. Audited with the room they came from.
    pub async fn move_to_room(
        &self,
        appointment_id: Uuid,
        room: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let room = match room.map(str::trim) {
            Some("") | None => None,
            Some(r) if r.chars().count() > 60 || r.chars().any(char::is_control) => {
                return Err(Error::validation(
                    "room must be 1 to 60 characters with no control characters",
                ));
            }
            Some(r) => Some(r.to_string()),
        };
        let ctx = ctx.clone();
        self.db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut appt = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    if appt.status != AppointmentStatus::Arrived {
                        return Err(Error::invalid_transition(format!(
                            "appointment {appointment_id} is {:?}; only a patient who has arrived can be moved",
                            appt.status
                        )));
                    }
                    let from = appt.current_room.clone();
                    appt.current_room = room.clone();
                    appt.updated_at = chrono::Utc::now();
                    let appt = AppointmentRepository::update(txn, &appt).await?;
                    AuditLogRepository::log_with(
                        txn,
                        "appointment",
                        appt.id,
                        "moved",
                        Some(serde_json::json!({ "room": from })),
                        Some(serde_json::json!({ "room": room })),
                        Some("moved_room"),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)
    }

    /// Everyone who has arrived and not yet been completed, longest waiting first, with the minutes
    /// they have been in the facility (CORE-4).
    pub async fn in_facility(&self) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT id, person_id, current_room, arrived_at, start_datetime, \
                        (extract(epoch FROM (now() - arrived_at)) / 60)::bigint AS waiting_minutes \
                 FROM appointments WHERE status = 'arrived' AND deleted_at IS NULL AND arrived_at IS NOT NULL \
                 ORDER BY arrived_at"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "appointment_id": r.try_get::<Uuid>("", "id").map_err(Error::Database)?,
                    "person_id": r.try_get::<Uuid>("", "person_id").map_err(Error::Database)?,
                    "room": r.try_get::<Option<String>>("", "current_room").map_err(Error::Database)?,
                    "arrived_at": r.try_get::<DateTime<Utc>>("", "arrived_at").map_err(Error::Database)?,
                    "appointment_start": r.try_get::<DateTime<Utc>>("", "start_datetime").map_err(Error::Database)?,
                    "waiting_minutes": r.try_get::<i64>("", "waiting_minutes").map_err(Error::Database)?.max(0),
                }))
            })
            .collect()
    }

    pub async fn check_in(&self, appointment_id: Uuid, ctx: &UserContext) -> Result<Appointment> {
        self.transition(appointment_id, AppointmentStatus::Arrived, "check_in", ctx)
            .await
    }

    pub async fn mark_no_show(
        &self,
        appointment_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut appt = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    appt.status = appt.status.try_transition_to(AppointmentStatus::NoShow)?;
                    appt.updated_at = chrono::Utc::now();
                    let appt = AppointmentRepository::update(txn, &appt).await?;
                    if let Some(sid) = appt.slot_id {
                        SlotRepository::update_status(txn, sid, SlotStatus::Free).await?;
                    }
                    // A miss returns the patient to the waiting list; it never removes them (CAP-7).
                    Self::sync_waitlist(txn, &appt, WaitlistStatus::Waiting, &ctx_clone).await?;
                    if let Some(class) = appt.release_class {
                        crate::standby::record_release(txn, &appt, class).await?;
                    }
                    // On a service a clinician has listed, a miss gets a person's follow-up (CAP-7).
                    crate::follow_up::on_no_show(txn, &appt, &ctx_clone).await?;
                    AuditLogRepository::log_with(
                        txn,
                        "appointment",
                        appt.id,
                        "no_show",
                        None,
                        None,
                        Some("no_show"),
                        Some(serde_json::json!({ "start": appt.start_datetime })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AppointmentNoShow",
                        &serde_json::json!({ "appointment_id": appt.id }),
                    )
                    .await?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)?;
        Ok(appt)
    }

    pub async fn complete(&self, appointment_id: Uuid, ctx: &UserContext) -> Result<Appointment> {
        self.transition(
            appointment_id,
            AppointmentStatus::Fulfilled,
            "complete",
            ctx,
        )
        .await
    }

    async fn transition(
        &self,
        appointment_id: Uuid,
        next: AppointmentStatus,
        action: &'static str,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut appt = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    appt.status = appt.status.try_transition_to(next)?;
                    appt.updated_at = chrono::Utc::now();
                    // Stamp the time (CORE-4). Arriving or finishing is the first time it is recorded.
                    match next {
                        AppointmentStatus::Arrived => appt.arrived_at = Some(appt.updated_at),
                        AppointmentStatus::Fulfilled => appt.completed_at = Some(appt.updated_at),
                        _ => {}
                    }
                    let appt = AppointmentRepository::update(txn, &appt).await?;
                    if next == AppointmentStatus::Fulfilled {
                        Self::sync_waitlist(txn, &appt, WaitlistStatus::Treated, &ctx_clone)
                            .await?;
                    }
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        appt.id,
                        action,
                        None,
                        None,
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        match next {
                            AppointmentStatus::Arrived => "AppointmentCheckedIn",
                            AppointmentStatus::Fulfilled => "AppointmentCompleted",
                            _ => "AppointmentStatusChanged",
                        },
                        &serde_json::json!({ "appointment_id": appt.id }),
                    )
                    .await?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)?;
        Ok(appt)
    }
}

/// The snake_case wire name of a unit-variant enum.
fn enum_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_appointment_proposed_to_booked() {
        let s = AppointmentStatus::Proposed
            .try_transition_to(AppointmentStatus::Booked)
            .unwrap();
        assert_eq!(s, AppointmentStatus::Booked);
    }

    #[test]
    fn test_slot_free_to_busy() {
        let s = SlotStatus::Free
            .try_transition_to(SlotStatus::Busy)
            .unwrap();
        assert_eq!(s, SlotStatus::Busy);
    }
}

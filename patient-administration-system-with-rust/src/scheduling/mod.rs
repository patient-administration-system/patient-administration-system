//! scheduling — Slot/Appointment booking service + recurring series.

pub mod series;

pub use series::{
    CreateSeriesInput, CreateSeriesResult, PreviewResult, SeriesService, SeriesWithOccurrences,
};

use sea_orm::{DatabaseConnection, DatabaseTransaction, TransactionTrait};
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
}

impl SchedulingService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
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
        AuditLogRepository::log(
            txn,
            "appointment",
            appt.id,
            "cancel",
            None,
            Some(serde_json::to_value(&appt).unwrap_or_default()),
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
            .map_err(unwrap_txn_err)?;
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
            .map_err(unwrap_txn_err)?;
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
            .map_err(unwrap_txn_err)?;
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
            .map_err(unwrap_txn_err)?;
        self.publish_cancelled(&old).await;
        self.publish_booked(&new).await;
        Ok((old, new))
    }

    /// Publish a domain event on this service's publisher (best effort, after a commit).
    pub(crate) async fn publish_event(&self, event: DomainEvent) -> Result<()> {
        self.publisher.publish(event).await
    }

    pub(crate) async fn publish_booked(&self, appt: &Appointment) {
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentBooked",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await;
    }

    pub(crate) async fn publish_cancelled(&self, appt: &Appointment) {
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
            .map_err(unwrap_txn_err)
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
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        appt.id,
                        "no_show",
                        None,
                        None,
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
            .map_err(unwrap_txn_err)?;
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
            .map_err(unwrap_txn_err)?;
        Ok(appt)
    }
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
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

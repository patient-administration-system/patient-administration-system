//! scheduling — Slot/Appointment booking service + recurring series.

pub mod series;

pub use series::{
    CreateSeriesInput, CreateSeriesResult, PreviewResult, SeriesService, SeriesWithOccurrences,
};

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

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
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

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

    pub async fn book_slot(
        &self,
        slot_id: Uuid,
        person_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
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
                    let mut appt =
                        Appointment::new(person_id, slot.start_datetime, slot.end_datetime);
                    appt.slot_id = Some(slot.id);
                    appt.status = appt.status.try_transition_to(AppointmentStatus::Booked)?;
                    let appt = AppointmentRepository::create(txn, &appt).await?;
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        appt.id,
                        "book",
                        None,
                        Some(serde_json::to_value(&appt).unwrap_or_default()),
                        &ctx_clone,
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
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentBooked",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await;
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
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
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
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        appt.id,
                        "cancel",
                        None,
                        Some(serde_json::to_value(&appt).unwrap_or_default()),
                        &ctx_clone,
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
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentCancelled",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await;
        Ok(appt)
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

//! Part of [`super`]: attendance.

use super::*;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Stmt, Transact};

/// Someone who has arrived and not been completed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct InFacility {
    pub appointment_id: Uuid,
    pub person_id: Uuid,
    #[schema(required = true)]
    pub room: Option<String>,
    pub arrived_at: DateTime<Utc>,
    pub appointment_start: DateTime<Utc>,
    pub waiting_minutes: i64,
}

impl SchedulingService {
    /// Record the barrier for an appointment that has already ended in a cancellation or a miss,
    /// for example after a welfare call. Refused for an appointment that is still to happen.
    pub async fn record_barrier(
        &self,
        appointment_id: Uuid,
        barrier: BarrierCode,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        self.record_barrier_if_unchanged(appointment_id, barrier, None, ctx)
            .await
    }

    /// [`Self::record_barrier`] that refuses with `PreconditionFailed` when the appointment has changed since
    /// the caller read it (OFH-10). `None` skips the comparison; the row is locked either way.
    pub async fn record_barrier_if_unchanged(
        &self,
        appointment_id: Uuid,
        barrier: BarrierCode,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        self.db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    let mut appt =
                        AppointmentRepository::lock_if_current(txn, appointment_id, expected)
                            .await?;
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
    }

    /// Where the patient is now (CORE-4). Only for someone who has arrived and not yet been completed;
    /// an empty room clears it. Audited with the room they came from.
    pub async fn move_to_room(
        &self,
        appointment_id: Uuid,
        room: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        self.move_to_room_if_unchanged(appointment_id, room, None, ctx)
            .await
    }

    /// [`Self::move_to_room`] that refuses with `PreconditionFailed` when the appointment has changed since
    /// the caller read it (OFH-10).
    pub async fn move_to_room_if_unchanged(
        &self,
        appointment_id: Uuid,
        room: Option<&str>,
        expected: Option<DateTime<Utc>>,
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
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    let mut appt =
                        AppointmentRepository::lock_if_current(txn, appointment_id, expected)
                            .await?;
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
    }

    /// Everyone who has arrived and not yet been completed, longest waiting first, with the minutes
    /// they have been in the facility (CORE-4).
    pub async fn in_facility(&self) -> Result<Vec<InFacility>> {
        let rows = self
            .db
            .all(Stmt::text("SELECT id, person_id, current_room, arrived_at, start_datetime, \
                        (extract(epoch FROM (now() - arrived_at)) / 60)::bigint AS waiting_minutes \
                 FROM appointments WHERE status = 'arrived' AND deleted_at IS NULL AND arrived_at IS NOT NULL \
                 ORDER BY arrived_at"
                    .to_string(),
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(InFacility {
                    appointment_id: r.col::<Uuid>("id")?,
                    person_id: r.col::<Uuid>("person_id")?,
                    room: r.col::<Option<String>>("current_room")?,
                    arrived_at: r.col::<DateTime<Utc>>("arrived_at")?,
                    appointment_start: r.col::<DateTime<Utc>>("start_datetime")?,
                    waiting_minutes: r.col::<i64>("waiting_minutes")?.max(0),
                })
            })
            .collect()
    }

    /// `Booked → Arrived`. Stamps `arrived_at`. No version is compared; see [`Self::check_in_if_unchanged`].
    pub async fn check_in(&self, appointment_id: Uuid, ctx: &UserContext) -> Result<Appointment> {
        self.check_in_if_unchanged(appointment_id, None, ctx).await
    }

    /// [`Self::check_in`] that refuses with `PreconditionFailed` when the appointment has changed (OFH-10).
    pub async fn check_in_if_unchanged(
        &self,
        appointment_id: Uuid,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        self.transition(
            appointment_id,
            AppointmentStatus::Arrived,
            "check_in",
            expected,
            ctx,
        )
        .await
    }

    /// `Booked → NoShow`: frees the slot, returns the patient to the waiting list (never removes them, CAP-7),
    /// records the release for the standby list and raises any follow-up task. No version is compared; see
    /// [`Self::mark_no_show_if_unchanged`].
    pub async fn mark_no_show(
        &self,
        appointment_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        self.mark_no_show_if_unchanged(appointment_id, None, ctx)
            .await
    }

    /// [`Self::mark_no_show`] that refuses with `PreconditionFailed` when the appointment has changed (OFH-10).
    pub async fn mark_no_show_if_unchanged(
        &self,
        appointment_id: Uuid,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    let mut appt =
                        AppointmentRepository::lock_if_current(txn, appointment_id, expected)
                            .await?;
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
            .await?;
        Ok(appt)
    }

    /// `Arrived → Fulfilled`. Stamps `completed_at` and marks the waiting-list entry treated. No version is
    /// compared; see [`Self::complete_if_unchanged`].
    pub async fn complete(&self, appointment_id: Uuid, ctx: &UserContext) -> Result<Appointment> {
        self.complete_if_unchanged(appointment_id, None, ctx).await
    }

    /// [`Self::complete`] that refuses with `PreconditionFailed` when the appointment has changed (OFH-10).
    pub async fn complete_if_unchanged(
        &self,
        appointment_id: Uuid,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        self.transition(
            appointment_id,
            AppointmentStatus::Fulfilled,
            "complete",
            expected,
            ctx,
        )
        .await
    }

    /// The shared body of check-in and complete: lock the appointment, compare `expected`, move the status
    /// along the state machine, stamp the time, sync the waiting list, and write the audit and outbox rows, all
    /// in one transaction. `action` is the audit action; `expected` `None` skips the comparison but still locks.
    pub(super) async fn transition(
        &self,
        appointment_id: Uuid,
        next: AppointmentStatus,
        action: &'static str,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx_clone = ctx.clone();
        let appt = self
            .db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    let mut appt =
                        AppointmentRepository::lock_if_current(txn, appointment_id, expected)
                            .await?;
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
            .await?;
        Ok(appt)
    }
}

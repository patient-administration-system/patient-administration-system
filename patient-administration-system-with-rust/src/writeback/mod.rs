//! Write-back (CAP-2): a patient's reply, arriving from outside, changes the appointment now.
//!
//! A reply is one of `confirm`, `cancel` or `rebook`, identified by the channel's own
//! `message_id`. Three guarantees:
//!
//! - **Idempotent.** The same `message_id` applies at most once; a second delivery returns the
//!   first one's outcome with `duplicate: true`. Reusing a `message_id` for a *different* reply is
//!   a conflict, not a silent replay.
//! - **Atomic.** The change and the record that it was made commit together, or neither does.
//! - **Honest about refusals.** A reply that cannot be applied for a deterministic reason (wrong
//!   patient, appointment already over, slot taken) is stored as `rejected` with the reason, and
//!   changes nothing. An unexpected failure is *not* stored, so a retry can still succeed.
//!
//! The sender is authenticated by the API's bearer token like every other caller; this module
//! additionally requires the reply to name both the appointment and the patient and refuses a
//! mismatch, so one patient's message cannot touch another's appointment.

use crate::db::row::RowExt;
use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, SqlErr, Statement,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::error::LogErr;
use crate::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancelledBy, ConversionState, Modality,
};
use crate::scheduling::{CancelDetails, SchedulingService};
use crate::streaming::DomainEvent;
use crate::{Error, Result};

/// The most replies one batch may carry.
pub const MAX_BATCH: usize = 1000;
const MAX_MESSAGE_ID: usize = 200;

/// The reject reason for a cancel held for a person to phone first (urgent or cancer pathways).
pub const HELD_FOR_STAFF: &str = "held_for_staff";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAction {
    Confirm,
    Cancel,
    Rebook,
    Convert,
}

impl ReplyAction {
    fn as_str(self) -> &'static str {
        match self {
            ReplyAction::Confirm => "confirm",
            ReplyAction::Cancel => "cancel",
            ReplyAction::Rebook => "rebook",
            ReplyAction::Convert => "convert",
        }
    }
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ReplyRequest {
    /// The channel's own id for the message. The idempotency key.
    pub message_id: String,
    pub action: ReplyAction,
    pub appointment_id: Uuid,
    /// The patient the reply is from. Must match the appointment's patient.
    pub person_id: Uuid,
    #[serde(default)]
    pub channel: Option<String>,
    /// For `cancel`: the barrier the patient gave, if any.
    #[serde(default)]
    pub barrier_code: Option<BarrierCode>,
    /// For `rebook`: the slot the patient chose.
    #[serde(default)]
    pub new_slot_id: Option<Uuid>,
    /// For `convert`: `video` or `phone` (CAP-14).
    #[serde(default)]
    pub modality: Option<Modality>,
    #[serde(default)]
    pub received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyStatus {
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplyResult {
    pub message_id: String,
    pub status: ReplyStatus,
    /// True when this `message_id` had already been handled and this is the stored outcome.
    pub duplicate: bool,
    pub reject_reason: Option<String>,
    /// The appointment as it now stands (after a cancel or rebook: the old, cancelled one).
    /// Absent when the reply was refused because it named the wrong patient or appointment.
    pub appointment: Option<Appointment>,
    /// For `rebook`: the new appointment.
    pub new_appointment: Option<Appointment>,
}

/// One row of a batch: the result, or why that row could not be processed at all.
#[derive(Debug, Serialize)]
pub struct BatchItem {
    pub index: usize,
    pub message_id: Option<String>,
    pub result: Option<ReplyResult>,
    pub error: Option<String>,
}

pub struct WriteBackService {
    db: DatabaseConnection,
    scheduling: Arc<SchedulingService>,
}

/// What happened inside the transaction, before the reply row is written.
struct Decision {
    status: ReplyStatus,
    reject_reason: Option<String>,
    appointment: Option<Appointment>,
    new_appointment: Option<Appointment>,
}

impl Decision {
    fn rejected(reason: impl Into<String>, appointment: Option<Appointment>) -> Self {
        Decision {
            status: ReplyStatus::Rejected,
            reject_reason: Some(reason.into()),
            appointment,
            new_appointment: None,
        }
    }
}

/// Which events to publish once the transaction has committed.
#[derive(Default)]
struct PostCommit {
    cancelled: Option<Appointment>,
    booked: Option<Appointment>,
    confirmed: Option<Uuid>,
}

impl WriteBackService {
    pub fn new(db: DatabaseConnection, scheduling: Arc<SchedulingService>) -> Self {
        Self { db, scheduling }
    }

    /// Check a request is well formed, before anything is looked up.
    pub fn validate(req: &ReplyRequest) -> Result<()> {
        let id = req.message_id.trim();
        if id.is_empty() || id.len() > MAX_MESSAGE_ID || id.chars().any(char::is_control) {
            return Err(Error::validation(
                "message_id must be 1 to 200 characters with no control characters",
            ));
        }
        match (req.action, req.new_slot_id) {
            (ReplyAction::Rebook, None) => {
                Err(Error::validation("a rebook reply must name new_slot_id"))
            }
            (ReplyAction::Confirm | ReplyAction::Cancel | ReplyAction::Convert, Some(_)) => Err(
                Error::validation("new_slot_id is only valid for a rebook reply"),
            ),
            _ => Ok(()),
        }?;
        if req.barrier_code.is_some()
            && !matches!(req.action, ReplyAction::Cancel | ReplyAction::Convert)
        {
            return Err(Error::validation(
                "barrier_code is only valid for a cancel or convert reply",
            ));
        }
        match (req.action, req.modality) {
            (ReplyAction::Convert, Some(Modality::Video | Modality::Phone)) => {}
            (ReplyAction::Convert, _) => {
                return Err(Error::validation(
                    "a convert reply must name modality video or phone",
                ));
            }
            (_, Some(_)) => {
                return Err(Error::validation(
                    "modality is only valid for a convert reply",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(action = ?req.action))]
    pub async fn apply(&self, req: ReplyRequest, ctx: &UserContext) -> Result<ReplyResult> {
        Self::validate(&req)?;
        if let Some(stored) = Self::stored(&self.db, &req).await? {
            return Ok(stored);
        }
        match self.apply_once(&req, ctx).await {
            Ok(r) => Ok(r),
            // Two deliveries of the same message raced; the loser rolled back whole, so there is
            // exactly one outcome to return.
            Err(Error::Database(e))
                if matches!(e.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) =>
            {
                Self::stored(&self.db, &req).await?.ok_or_else(|| {
                    Error::internal("duplicate message_id reported but no stored reply found")
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Process many replies, each independently: one bad row does not stop the others.
    pub async fn apply_batch(
        &self,
        reqs: Vec<ReplyRequest>,
        ctx: &UserContext,
    ) -> Result<Vec<BatchItem>> {
        if reqs.len() > MAX_BATCH {
            return Err(Error::validation(format!(
                "a batch may carry at most {MAX_BATCH} replies"
            )));
        }
        let mut out = Vec::with_capacity(reqs.len());
        for (index, req) in reqs.into_iter().enumerate() {
            let message_id = Some(req.message_id.clone());
            match self.apply(req, ctx).await {
                Ok(result) => out.push(BatchItem {
                    index,
                    message_id,
                    result: Some(result),
                    error: None,
                }),
                Err(e) => out.push(BatchItem {
                    index,
                    message_id,
                    result: None,
                    error: Some(e.to_string()),
                }),
            }
        }
        Ok(out)
    }

    /// The stored outcome for this message, if it was handled before.
    async fn stored<C: ConnectionTrait>(
        conn: &C,
        req: &ReplyRequest,
    ) -> Result<Option<ReplyResult>> {
        let row = conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT action, appointment_id, person_id, status, reject_reason, new_appointment_id \
                 FROM inbound_replies WHERE message_id = $1",
                [req.message_id.trim().into()],
            ))
            .await
            .map_err(Error::Database)?;
        let Some(row) = row else { return Ok(None) };
        let action: String = row.col("action")?;
        let appointment_id: Uuid = row.col("appointment_id")?;
        let person_id: Uuid = row.col("person_id")?;
        if action != req.action.as_str()
            || appointment_id != req.appointment_id
            || person_id != req.person_id
        {
            return Err(Error::conflict(
                "message_id was already used for a different reply; a message id identifies one reply",
            ));
        }
        let status: String = row.col("status")?;
        let reject_reason: Option<String> = row.col("reject_reason")?;
        let new_id: Option<Uuid> = row.col("new_appointment_id")?;
        let status = if status == "applied" {
            ReplyStatus::Applied
        } else {
            ReplyStatus::Rejected
        };
        // A rejection for the wrong patient or an unknown appointment shows nothing about it.
        let reveal = !matches!(
            reject_reason.as_deref(),
            Some("person_mismatch" | "unknown_appointment")
        );
        let appointment = if reveal {
            AppointmentRepository::find_by_id(conn, appointment_id).await?
        } else {
            None
        };
        let new_appointment = match new_id {
            Some(id) => AppointmentRepository::find_by_id(conn, id).await?,
            None => None,
        };
        Ok(Some(ReplyResult {
            message_id: req.message_id.trim().to_string(),
            status,
            duplicate: true,
            reject_reason,
            appointment,
            new_appointment,
        }))
    }

    async fn apply_once(&self, req: &ReplyRequest, ctx: &UserContext) -> Result<ReplyResult> {
        let request = req.clone();
        let ctx = ctx.clone();
        let (result, post) = self
            .db
            .transaction::<_, (ReplyResult, PostCommit), Error>(move |txn| {
                Box::pin(async move { Self::decide_and_record(txn, &request, &ctx).await })
            })
            .await
            .map_err(Error::from)?;
        if let Some(a) = &post.cancelled {
            self.scheduling.publish_cancelled(a).await;
        }
        if let Some(a) = &post.booked {
            self.scheduling.publish_booked(a).await;
        }
        if let Some(id) = post.confirmed {
            self.scheduling.after_confirmed(id).await;
            self.scheduling
                .publish_event(DomainEvent::new(
                    "AppointmentConfirmed",
                    serde_json::json!({ "appointment_id": id }),
                ))
                .await
                .log_warn("in-process event");
        }
        Ok(result)
    }

    async fn decide_and_record(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        ctx: &UserContext,
    ) -> Result<(ReplyResult, PostCommit)> {
        // Serialize deliveries of the same message. Without this, two simultaneous deliveries both
        // see "not handled yet", both do the work, and one fails after the other commits. The lock
        // lasts until this transaction ends; the second delivery then finds the first one's outcome.
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            [req.message_id.trim().into()],
        ))
        .await
        .map_err(Error::Database)?;
        if let Some(already) = Self::stored(txn, req).await? {
            return Ok((already, PostCommit::default()));
        }
        let mut post = PostCommit::default();
        let appt = AppointmentRepository::find_by_id(txn, req.appointment_id).await?;
        let decision = match appt {
            None => Decision::rejected("unknown_appointment", None),
            Some(a) if a.person_id != req.person_id => Decision::rejected("person_mismatch", None),
            Some(a) => Self::act(txn, req, a, ctx, &mut post).await?,
        };
        let new_id = decision.new_appointment.as_ref().map(|a| a.id);
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO inbound_replies \
               (id, message_id, action, appointment_id, person_id, channel, barrier_code, new_slot_id, \
                status, reject_reason, new_appointment_id, received_at, modality) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
            [
                Uuid::new_v4().into(),
                req.message_id.trim().into(),
                req.action.as_str().into(),
                req.appointment_id.into(),
                req.person_id.into(),
                req.channel.clone().into(),
                req.barrier_code.and_then(|b| serde_json::to_value(b).ok().and_then(|v| v.as_str().map(String::from))).into(),
                req.new_slot_id.into(),
                (if decision.status == ReplyStatus::Applied { "applied" } else { "rejected" }).into(),
                decision.reject_reason.clone().into(),
                new_id.into(),
                req.received_at.unwrap_or_else(Utc::now).into(),
                req.modality.map(|m| m.as_str().to_string()).into(),
            ],
        ))
        .await
        .map_err(Error::Database)?;
        Ok((
            ReplyResult {
                message_id: req.message_id.trim().to_string(),
                status: decision.status,
                duplicate: false,
                reject_reason: decision.reject_reason,
                appointment: decision.appointment,
                new_appointment: decision.new_appointment,
            },
            post,
        ))
    }

    /// Apply the action to an appointment known to belong to the replying patient.
    async fn act(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        appt: Appointment,
        ctx: &UserContext,
        post: &mut PostCommit,
    ) -> Result<Decision> {
        match req.action {
            ReplyAction::Confirm => Self::act_confirm(txn, req, appt, ctx, post).await,
            ReplyAction::Cancel => Self::act_cancel(txn, req, appt, ctx, post).await,
            ReplyAction::Convert => Self::act_convert(txn, req, appt, ctx).await,
            ReplyAction::Rebook => Self::act_rebook(txn, req, appt, ctx, post).await,
        }
    }

    /// Confirm a booked appointment (once; a second confirm is still a success).
    async fn act_confirm(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        appt: Appointment,
        ctx: &UserContext,
        post: &mut PostCommit,
    ) -> Result<Decision> {
        if appt.status != AppointmentStatus::Booked {
            return Ok(Decision::rejected(
                format!("not_confirmable:{:?}", appt.status).to_lowercase(),
                Some(appt),
            ));
        }
        if appt.confirmed_at.is_some() {
            // Already confirmed by an earlier message: nothing to change, and still success.
            return Ok(Decision {
                status: ReplyStatus::Applied,
                reject_reason: None,
                appointment: Some(appt),
                new_appointment: None,
            });
        }
        let mut a = appt;
        let now = Utc::now();
        a.confirmed_at = Some(now);
        a.updated_at = now;
        let a = AppointmentRepository::update(txn, &a).await?;
        Self::audit(txn, &a, "confirm", req, ctx).await?;
        OutboxRepository::publish(
            txn,
            "AppointmentConfirmed",
            &serde_json::json!({ "appointment_id": a.id }),
        )
        .await?;
        post.confirmed = Some(a.id);
        Ok(Decision {
            status: ReplyStatus::Applied,
            reject_reason: None,
            appointment: Some(a),
            new_appointment: None,
        })
    }

    /// Cancel on the patient's behalf, unless the service is listed for human follow-up (then a person
    /// phones first and the slot is kept).
    async fn act_cancel(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        appt: Appointment,
        ctx: &UserContext,
        post: &mut PostCommit,
    ) -> Result<Decision> {
        if !matches!(
            appt.status,
            AppointmentStatus::Booked | AppointmentStatus::Proposed
        ) {
            return Ok(Decision::rejected(
                format!("not_cancellable:{:?}", appt.status).to_lowercase(),
                Some(appt),
            ));
        }
        // On a service a clinician has listed (urgent or cancer pathways), a patient's own cancel
        // is not applied: the slot is kept and a person phones first (CAP-7, FR-58).
        if crate::follow_up::service_is_listed(txn, appt.slot_id).await? {
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                 VALUES ($1,$2,$3,'urgent_pathway', now()) ON CONFLICT (appointment_id, reason) DO NOTHING",
                [Uuid::new_v4().into(), appt.id.into(), appt.person_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
            Self::audit(txn, &appt, "cancel_held", req, ctx).await?;
            return Ok(Decision::rejected(HELD_FOR_STAFF, Some(appt)));
        }
        let a = SchedulingService::cancel_tx(
            txn,
            appt.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: req.barrier_code,
                legacy_reason: None,
            },
            ctx,
        )
        .await?;
        Self::audit(txn, &a, "cancel", req, ctx).await?;
        post.cancelled = Some(a.clone());
        Ok(Decision {
            status: ReplyStatus::Applied,
            reject_reason: None,
            appointment: Some(a),
            new_appointment: None,
        })
    }

    /// Ask for a video or phone visit instead of in person (CAP-14); a person reviews it.
    async fn act_convert(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        appt: Appointment,
        ctx: &UserContext,
    ) -> Result<Decision> {
        let modality = req
            .modality
            .ok_or_else(|| Error::validation("convert needs a modality"))?;
        if appt.status != AppointmentStatus::Booked {
            return Ok(Decision::rejected(
                format!("not_convertible:{:?}", appt.status).to_lowercase(),
                Some(appt),
            ));
        }
        if appt.start_datetime <= Utc::now() {
            return Ok(Decision::rejected("already_started", Some(appt)));
        }
        if appt.modality != Modality::InPerson
            && appt.conversion_state != Some(ConversionState::Vetoed)
        {
            // Asking for the same thing twice is success; asking for something else is not.
            return Ok(if appt.modality == modality {
                Decision {
                    status: ReplyStatus::Applied,
                    reject_reason: None,
                    appointment: Some(appt),
                    new_appointment: None,
                }
            } else {
                Decision::rejected("already_converted", Some(appt))
            });
        }
        if !crate::virtual_visits::service_is_virtual_ok(txn, appt.slot_id).await? {
            return Ok(Decision::rejected("not_virtual_ok", Some(appt)));
        }
        let mut a = appt;
        let now = Utc::now();
        a.modality = modality;
        a.conversion_state = Some(ConversionState::PendingReview);
        a.conversion_barrier_code = req.barrier_code;
        a.converted_at = Some(now);
        a.updated_at = now;
        let a = AppointmentRepository::update(txn, &a).await?;
        Self::audit(txn, &a, "convert", req, ctx).await?;
        OutboxRepository::publish(
            txn,
            "AppointmentConverted",
            &serde_json::json!({
                "appointment_id": a.id, "modality": modality.as_str(),
                "barrier_code": req.barrier_code,
            }),
        )
        .await?;
        Ok(Decision {
            status: ReplyStatus::Applied,
            reject_reason: None,
            appointment: Some(a),
            new_appointment: None,
        })
    }

    /// Move to another slot the patient chose (CAP-5).
    async fn act_rebook(
        txn: &DatabaseTransaction,
        req: &ReplyRequest,
        appt: Appointment,
        ctx: &UserContext,
        post: &mut PostCommit,
    ) -> Result<Decision> {
        let new_slot = req
            .new_slot_id
            .ok_or_else(|| Error::validation("rebook needs new_slot_id"))?;
        match SchedulingService::rebook_tx(txn, appt.id, new_slot, ctx).await {
            Ok((old, new)) => {
                Self::audit(txn, &old, "rebook", req, ctx).await?;
                post.cancelled = Some(old.clone());
                post.booked = Some(new.clone());
                Ok(Decision {
                    status: ReplyStatus::Applied,
                    reject_reason: None,
                    appointment: Some(old),
                    new_appointment: Some(new),
                })
            }
            // rebook_tx checks everything before it writes, so a refusal changed nothing.
            Err(Error::Conflict(_)) | Err(Error::NotFound(_)) => {
                Ok(Decision::rejected("slot_unavailable", Some(appt)))
            }
            Err(Error::InvalidStateTransition(_)) => {
                Ok(Decision::rejected("not_movable", Some(appt)))
            }
            Err(e) => Err(e),
        }
    }

    async fn audit(
        txn: &DatabaseTransaction,
        a: &Appointment,
        action: &str,
        req: &ReplyRequest,
        ctx: &UserContext,
    ) -> Result<()> {
        let facts = serde_json::json!({
            "message_id": req.message_id.trim(),
            "channel": req.channel,
            "action": action,
        });
        // The reason a patient gave when there is one, else what they asked for (CAP-6).
        let reason = req
            .barrier_code
            .map_or_else(|| format!("patient_{action}"), |b| b.as_str().to_string());
        AuditLogRepository::log_with(
            txn,
            "appointment",
            a.id,
            &format!("reply_{action}"),
            None,
            Some(facts.clone()),
            Some(&reason),
            Some(facts),
            ctx,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(action: ReplyAction) -> ReplyRequest {
        ReplyRequest {
            message_id: "wamid.123".into(),
            action,
            appointment_id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            channel: Some("whatsapp".into()),
            barrier_code: None,
            new_slot_id: None,
            modality: None,
            received_at: None,
        }
    }

    #[test]
    fn test_validate_accepts_each_well_formed_reply() {
        assert!(WriteBackService::validate(&req(ReplyAction::Confirm)).is_ok());
        assert!(WriteBackService::validate(&req(ReplyAction::Cancel)).is_ok());
        let mut r = req(ReplyAction::Rebook);
        r.new_slot_id = Some(Uuid::new_v4());
        assert!(WriteBackService::validate(&r).is_ok());
    }

    #[test]
    fn test_validate_refuses_bad_message_ids() {
        for bad in ["", "   ", "has\ncontrol", &"x".repeat(MAX_MESSAGE_ID + 1)] {
            let mut r = req(ReplyAction::Confirm);
            r.message_id = bad.into();
            assert!(WriteBackService::validate(&r).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn test_validate_ties_new_slot_to_rebook_and_barrier_to_cancel() {
        assert!(
            WriteBackService::validate(&req(ReplyAction::Rebook)).is_err(),
            "rebook needs a slot"
        );
        let mut c = req(ReplyAction::Confirm);
        c.new_slot_id = Some(Uuid::new_v4());
        assert!(
            WriteBackService::validate(&c).is_err(),
            "a slot only belongs on a rebook"
        );
        let mut b = req(ReplyAction::Confirm);
        b.barrier_code = Some(BarrierCode::Work);
        assert!(
            WriteBackService::validate(&b).is_err(),
            "a barrier only belongs on a cancel"
        );
        let mut ok = req(ReplyAction::Cancel);
        ok.barrier_code = Some(BarrierCode::Work);
        assert!(WriteBackService::validate(&ok).is_ok());
    }
}

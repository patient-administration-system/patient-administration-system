//! Human follow-up after a missed appointment (CAP-7).
//!
//! A clinician lists the services whose misses must always get a person's attention
//! (`follow_up_policies`, with `approved_by`). Marking an appointment on such a service as a no-show
//! creates one `follow_up_tasks` row, due within `PAS_FOLLOW_UP_HOURS` (default 48), in the same
//! transaction. The task is closed only by a person recording an outcome and their name.
//!
//! The rule this module exists to keep: **a miss triggers contact, never discharge.** Nothing here removes
//! the patient from a waiting list, discharges them, or stops their referral-to-treatment clock; the
//! no-show path returns the waiting-list entry to `waiting`, as it always has.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::appointment::Appointment;
use crate::{Error, Result};

crate::string_enum! {
    /// What happened on a follow-up call.
    pub enum TaskOutcome {
        ReachedRebooked = "reached_rebooked",
        ReachedWillAttend = "reached_will_attend",
        ReachedNoLongerNeeded = "reached_no_longer_needed",
        NoAnswer = "no_answer",
        WrongNumber = "wrong_number",
        DeclinedHelp = "declined_help",
        ClinicalEscalation = "clinical_escalation",
    }
}

fn due_after() -> Duration {
    Duration::hours(env::int("PAS_FOLLOW_UP_HOURS"))
}

/// Is the appointment's service one a clinician has listed for human follow-up?
pub(crate) async fn service_is_listed<C: Conn>(conn: &C, slot_id: Option<Uuid>) -> Result<bool> {
    let Some(slot_id) = slot_id else {
        return Ok(false);
    };
    Ok(conn
        .one(Stmt::new(
            "SELECT 1 AS ok FROM slots sl JOIN schedules sc ON sc.id = sl.schedule_id \
             JOIN follow_up_policies p ON p.service_type = sc.service_type WHERE sl.id = $1",
            [slot_id.into()],
        ))
        .await?
        .is_some())
}

/// Called inside the no-show transaction. Creates the task when the appointment's service is listed; a
/// second call for the same appointment does nothing.
pub(crate) async fn on_no_show<C: Conn>(
    conn: &C,
    appt: &Appointment,
    ctx: &UserContext,
) -> Result<()> {
    let Some(slot_id) = appt.slot_id else {
        return Ok(());
    };
    let Some(row) = conn
        .one(Stmt::new(
            "SELECT sc.service_type FROM slots sl JOIN schedules sc ON sc.id = sl.schedule_id \
             JOIN follow_up_policies p ON p.service_type = sc.service_type WHERE sl.id = $1",
            [slot_id.into()],
        ))
        .await?
    else {
        return Ok(());
    };
    let service: String = row.col("service_type")?;
    let id = Uuid::new_v4();
    let due = Utc::now() + due_after();
    let made = conn
        .run(Stmt::new(
            "INSERT INTO follow_up_tasks (id, appointment_id, person_id, service_type, due_at) \
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT (appointment_id) DO NOTHING",
            [
                id.into(),
                appt.id.into(),
                appt.person_id.into(),
                service.clone().into(),
                due.into(),
            ],
        ))
        .await?;
    if made.rows_affected() == 0 {
        return Ok(());
    }
    AuditLogRepository::log(
        conn,
        "appointment",
        appt.id,
        "follow_up_task_created",
        None,
        Some(serde_json::json!({ "task_id": id, "service_type": service, "due_at": due })),
        ctx,
    )
    .await?;
    OutboxRepository::publish(
        conn,
        "FollowUpTaskCreated",
        &serde_json::json!({ "task_id": id, "appointment_id": appt.id, "due_at": due }),
    )
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Policy {
    pub service_type: String,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Task {
    pub id: Uuid,
    pub appointment_id: Uuid,
    pub person_id: Uuid,
    pub service_type: String,
    pub due_at: DateTime<Utc>,
    pub overdue: bool,
    pub appointment_start: DateTime<Utc>,
}

pub struct FollowUpService {
    db: Pool,
}

impl FollowUpService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// List a service. `approved_by` must name a person.
    pub async fn approve_policy(
        &self,
        service_type: &str,
        approved_by: &str,
        ctx: &UserContext,
    ) -> Result<Policy> {
        let service_type = service_type.trim();
        let approved_by = approved_by.trim();
        if service_type.is_empty() || service_type.len() > 200 {
            return Err(Error::validation(
                "service_type must be 1 to 200 characters",
            ));
        }
        let approved_by = crate::validation::named_person(
            approved_by,
            "a follow-up policy needs a named clinician's approval (approved_by)",
        )?;
        self.db
            .run(Stmt::new(
                "INSERT INTO follow_up_policies (service_type, approved_by) VALUES ($1,$2) \
                 ON CONFLICT (service_type) DO UPDATE SET approved_by = $2, approved_at = now()",
                [service_type.into(), approved_by.into()],
            ))
            .await?;
        AuditLogRepository::log(
            &self.db,
            "follow_up_policy",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, service_type.as_bytes()),
            "approved",
            None,
            Some(serde_json::json!({ "service_type": service_type, "approved_by": approved_by })),
            ctx,
        )
        .await?;
        self.policies()
            .await?
            .into_iter()
            .find(|p| p.service_type == service_type)
            .ok_or_else(|| Error::internal("approved policy not found"))
    }

    pub async fn policies(&self) -> Result<Vec<Policy>> {
        let rows = self
            .db
            .all(Stmt::text("SELECT service_type, approved_by, approved_at FROM follow_up_policies ORDER BY service_type",
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(Policy {
                    service_type: r.col("service_type")?,
                    approved_by: r.col("approved_by")?,
                    approved_at: r.col("approved_at")?,
                })
            })
            .collect()
    }

    /// Open tasks, most overdue first.
    pub async fn open_tasks(&self, rows: impl Into<crate::paging::Rows>) -> Result<Vec<Task>> {
        let rows = rows.into();
        let rows = self
            .db
            .all(Stmt::new("SELECT t.id, t.appointment_id, t.person_id, t.service_type, t.due_at, a.start_datetime \
                 FROM follow_up_tasks t JOIN appointments a ON a.id = t.appointment_id \
                 WHERE t.closed_at IS NULL ORDER BY t.due_at, t.id LIMIT $1 OFFSET $2",
                [rows.fetch().into(), rows.offset().into()],
            ))
.await?;
        let now = Utc::now();
        rows.iter()
            .map(|r| {
                let due_at: DateTime<Utc> = r.col("due_at")?;
                Ok(Task {
                    id: r.col("id")?,
                    appointment_id: r.col("appointment_id")?,
                    person_id: r.col("person_id")?,
                    service_type: r.col("service_type")?,
                    due_at,
                    overdue: due_at < now,
                    appointment_start: r.col("start_datetime")?,
                })
            })
            .collect()
    }

    /// A person records what happened. Refused without a valid outcome and a name, and for a task that is
    /// not open. Closing is the only way a task ends.
    pub async fn close_task(
        &self,
        id: Uuid,
        outcome: &str,
        closed_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let outcome: TaskOutcome = outcome.parse().map_err(|_| {
            Error::validation(format!(
                "outcome must be one of {}",
                TaskOutcome::NAMES.join(", ")
            ))
        })?;
        let closed_by = crate::validation::named_person(
            closed_by,
            "a follow-up is closed by a named person (closed_by)",
        )?;
        let row = self
            .db
            .one(Stmt::new(
                "UPDATE follow_up_tasks SET outcome = $2, closed_by = $3, closed_at = now() \
                 WHERE id = $1 AND closed_at IS NULL RETURNING appointment_id",
                [id.into(), outcome.as_str().into(), closed_by.into()],
            ))
            .await?;
        let Some(row) = row else {
            return Err(Error::conflict("that follow-up task is not open"));
        };
        let appointment_id: Uuid = row.col("appointment_id")?;
        AuditLogRepository::log(
            &self.db,
            "appointment",
            appointment_id,
            "follow_up_task_closed",
            None,
            Some(serde_json::json!({ "task_id": id, "outcome": outcome.as_str(), "closed_by": closed_by })),
            ctx,
        )
        .await
    }
}

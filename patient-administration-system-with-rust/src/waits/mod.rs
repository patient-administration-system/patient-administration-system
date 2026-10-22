//! Wait-time and lead-time data (CAP-8).
//!
//! - **Lead time** of an appointment is the time from when it was booked to when it starts. It is computed
//!   from the booking time, never stored, so it cannot drift.
//! - **Weeks waited** on a pathway is the referral-to-treatment clock's own figure (`compute_active_weeks`).
//! - **Long waiters** are active pathways over N weeks (`PAS_CHECK_WEEKS`, default 26) with no
//!   "do you still need this?" check in the last D days (`PAS_CHECK_REPEAT_DAYS`, default 90).
//! - **The answer** is recorded and audited by a named person. Answering "no longer needed" changes nothing
//!   else: it does not remove the patient from a list or stop their clock. A person decides what follows.

use crate::db::row::RowExt;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::rtt::RttRepository;
use crate::models::rtt::compute_active_weeks;
use crate::{Error, Result};

pub const ANSWERS: [&str; 4] = ["still_needed", "no_longer_needed", "unsure", "no_reply"];

#[derive(Debug, Clone, Serialize)]
pub struct LeadTime {
    pub appointment_id: Uuid,
    pub booked_at: DateTime<Utc>,
    pub starts_at: DateTime<Utc>,
    /// Whole days from booking to the visit (negative never: an appointment starts after it is made).
    pub lead_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathwayWait {
    pub pathway_id: Uuid,
    pub target_service: String,
    pub weeks_waited: u32,
    pub last_check_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PatientWaits {
    pub person_id: Uuid,
    pub appointments: Vec<LeadTime>,
    pub pathways: Vec<PathwayWait>,
}

pub struct WaitsService {
    db: DatabaseConnection,
}

impl WaitsService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    fn lead(appointment_id: Uuid, booked_at: DateTime<Utc>, starts_at: DateTime<Utc>) -> LeadTime {
        LeadTime {
            appointment_id,
            booked_at,
            starts_at,
            lead_days: (starts_at - booked_at).num_days().max(0),
        }
    }

    pub async fn lead_time(&self, appointment_id: Uuid) -> Result<LeadTime> {
        let r = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT created_at, start_datetime FROM appointments WHERE id = $1 AND deleted_at IS NULL",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
        Ok(Self::lead(
            appointment_id,
            r.col("created_at")?,
            r.col("start_datetime")?,
        ))
    }

    async fn weeks(&self, pathway_id: Uuid) -> Result<u32> {
        let events = RttRepository::list_events_for_pathway(&self.db, pathway_id).await?;
        Ok(compute_active_weeks(&events, Utc::now()))
    }

    /// A patient's appointments (lead time of each, not cancelled) and pathways (weeks waited).
    pub async fn patient_waits(&self, person_id: Uuid) -> Result<PatientWaits> {
        let appts = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, created_at, start_datetime FROM appointments \
                 WHERE person_id = $1 AND deleted_at IS NULL AND status NOT IN ('cancelled','no_show') \
                 ORDER BY start_datetime",
                [person_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let appointments = appts
            .iter()
            .map(|r| {
                Ok(Self::lead(
                    r.col("id")?,
                    r.col("created_at")?,
                    r.col("start_datetime")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let paths = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT p.id, p.target_service, \
                        (SELECT max(asked_at) FROM still_needed_checks c WHERE c.pathway_id = p.id) AS last_check \
                 FROM rtt_pathways p WHERE p.person_id = $1 AND p.stopped_at IS NULL ORDER BY p.started_at",
                [person_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut pathways = Vec::new();
        for r in &paths {
            let id: Uuid = r.col("id")?;
            pathways.push(PathwayWait {
                pathway_id: id,
                target_service: r.col("target_service")?,
                weeks_waited: self.weeks(id).await?,
                last_check_at: r.col("last_check")?,
            });
        }
        Ok(PatientWaits {
            person_id,
            appointments,
            pathways,
        })
    }

    /// Active pathways over `weeks` waited with no check asked in the last `repeat_days`, longest first.
    pub async fn due_for_check(
        &self,
        weeks: Option<i64>,
        repeat_days: Option<i64>,
    ) -> Result<Vec<(Uuid, PathwayWait)>> {
        let weeks = weeks.unwrap_or_else(|| env::int("PAS_CHECK_WEEKS"));
        let repeat = repeat_days.unwrap_or_else(|| env::int("PAS_CHECK_REPEAT_DAYS"));
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT p.id, p.person_id, p.target_service, \
                        (SELECT max(asked_at) FROM still_needed_checks c WHERE c.pathway_id = p.id) AS last_check \
                 FROM rtt_pathways p WHERE p.stopped_at IS NULL \
                   AND NOT EXISTS (SELECT 1 FROM still_needed_checks c WHERE c.pathway_id = p.id \
                                   AND c.asked_at > now() - make_interval(days => $1::int)) \
                 ORDER BY p.started_at LIMIT 5000",
                [(repeat as i32).into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in &rows {
            let id: Uuid = r.col("id")?;
            let w = self.weeks(id).await?;
            if i64::from(w) >= weeks {
                out.push((
                    r.col("person_id")?,
                    PathwayWait {
                        pathway_id: id,
                        target_service: r.col("target_service")?,
                        weeks_waited: w,
                        last_check_at: r.col("last_check")?,
                    },
                ));
            }
        }
        out.sort_by_key(|(_, p)| std::cmp::Reverse(p.weeks_waited));
        Ok(out)
    }

    /// A person asks a patient whether they still need this. Returns the check's id.
    pub async fn ask(&self, pathway_id: Uuid, asked_by: &str, ctx: &UserContext) -> Result<Uuid> {
        let asked_by =
            crate::validation::named_person(asked_by, "a check names who asked (asked_by)")?;
        let person = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM rtt_pathways WHERE id = $1 AND stopped_at IS NULL",
                [pathway_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("open pathway {pathway_id}")))?;
        let person_id: Uuid = person.col("person_id")?;
        let id = Uuid::new_v4();
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO still_needed_checks (id, pathway_id, person_id, asked_by) VALUES ($1,$2,$3,$4)",
                [id.into(), pathway_id.into(), person_id.into(), asked_by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "rtt_pathway",
            pathway_id,
            "still_needed_asked",
            None,
            Some(serde_json::json!({ "check_id": id, "asked_by": asked_by })),
            ctx,
        )
        .await?;
        Ok(id)
    }

    /// Record the answer. Audited. Changes nothing else: the patient stays on every list and the clock runs.
    pub async fn answer(
        &self,
        check_id: Uuid,
        answer: &str,
        recorded_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !ANSWERS.contains(&answer) {
            return Err(Error::validation(format!(
                "answer must be one of {}",
                ANSWERS.join(", ")
            )));
        }
        let recorded_by = crate::validation::named_person(
            recorded_by,
            "an answer is recorded by a named person (recorded_by)",
        )?;
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE still_needed_checks SET answer = $2, recorded_by = $3, answered_at = now() \
                 WHERE id = $1 AND answer IS NULL RETURNING pathway_id",
                [check_id.into(), answer.into(), recorded_by.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::conflict("that check is unknown or already answered"))?;
        let pathway_id: Uuid = row.col("pathway_id")?;
        AuditLogRepository::log_with(
            &self.db,
            "rtt_pathway",
            pathway_id,
            "still_needed_answered",
            None,
            Some(serde_json::json!({ "check_id": check_id, "answer": answer, "recorded_by": recorded_by })),
            Some(answer),
            None,
            ctx,
        )
        .await
    }
}

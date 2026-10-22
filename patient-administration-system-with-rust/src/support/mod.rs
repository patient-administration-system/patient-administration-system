//! Support flags that can only add help (CAP-10, NFR-11).
//!
//! A flag says "a person should phone this patient" or "refer this patient to a navigator". Nothing reads
//! a flag to change what a patient is offered: rebooking options, standby candidates, reminders and slots
//! are computed without looking at this table, and a schema guard test fails if any column anywhere
//! scores, ranks, penalises or suppresses contact. Resolving a flag needs a named person and an outcome.

use crate::db::row::RowExt;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const KINDS: [&str; 2] = ["needs_support_call", "navigator_referral"];
pub const OUTCOMES: [&str; 4] = ["helped", "no_answer", "declined_help", "not_needed"];

#[derive(Debug, Clone, Serialize)]
pub struct SupportFlag {
    pub id: Uuid,
    pub person_id: Uuid,
    pub appointment_id: Option<Uuid>,
    pub kind: String,
    pub reason_code: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

pub struct SupportService {
    db: DatabaseConnection,
}

impl SupportService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn add(
        &self,
        person_id: Uuid,
        kind: &str,
        appointment_id: Option<Uuid>,
        reason_code: Option<&str>,
        created_by: &str,
        ctx: &UserContext,
    ) -> Result<Uuid> {
        if !KINDS.contains(&kind) {
            return Err(Error::validation(format!(
                "kind must be one of {}",
                KINDS.join(", ")
            )));
        }
        let created_by = crate::validation::named_person(
            created_by,
            "a support flag names who raised it (created_by)",
        )?;
        if let Some(r) = reason_code
            && (r.is_empty()
                || r.len() > 60
                || !r.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        {
            return Err(Error::validation(
                "reason_code is a short snake_case code, not free text",
            ));
        }
        let id = Uuid::new_v4();
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO support_flags (id, person_id, appointment_id, kind, reason_code, created_by) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
                [
                    id.into(),
                    person_id.into(),
                    appointment_id.into(),
                    kind.into(),
                    reason_code.map(String::from).into(),
                    created_by.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log_with(
            &self.db,
            "support_flag",
            id,
            "raised",
            None,
            Some(serde_json::json!({ "kind": kind, "appointment_id": appointment_id })),
            reason_code,
            None,
            ctx,
        )
        .await?;
        Ok(id)
    }

    /// Open flags, oldest first.
    pub async fn open(&self, limit: i64) -> Result<Vec<SupportFlag>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, person_id, appointment_id, kind, reason_code, created_by, created_at \
                 FROM support_flags WHERE resolved_at IS NULL ORDER BY created_at LIMIT $1",
                [limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(SupportFlag {
                    id: r.col("id")?,
                    person_id: r.col("person_id")?,
                    appointment_id: r.col("appointment_id")?,
                    kind: r.col("kind")?,
                    reason_code: r.col("reason_code")?,
                    created_by: r.col("created_by")?,
                    created_at: r.col("created_at")?,
                })
            })
            .collect()
    }

    /// A named person records the outcome. Refused for an invalid outcome, no name, or a flag not open.
    pub async fn resolve(
        &self,
        id: Uuid,
        outcome: &str,
        resolved_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !OUTCOMES.contains(&outcome) {
            return Err(Error::validation(format!(
                "outcome must be one of {}",
                OUTCOMES.join(", ")
            )));
        }
        let resolved_by = crate::validation::named_person(
            resolved_by,
            "a flag is resolved by a named person (resolved_by)",
        )?;
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE support_flags SET resolved_at = now(), resolved_by = $3, outcome = $2 \
                 WHERE id = $1 AND resolved_at IS NULL",
                [id.into(), outcome.into(), resolved_by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::conflict("that support flag is not open"));
        }
        AuditLogRepository::log_with(
            &self.db,
            "support_flag",
            id,
            "resolved",
            None,
            Some(serde_json::json!({ "outcome": outcome, "resolved_by": resolved_by })),
            Some(outcome),
            None,
            ctx,
        )
        .await
    }
}

//! Identity proofing (DTAC-9).
//!
//! The patient portal serves a signed-in account only after staff have recorded that they checked who the
//! account belongs to, and which patient it is. [`IdentityService::check`] answers one question, "is this
//! account proofed as this patient?", and the portal asks it before every patient page. It answers yes only
//! for a live (unrevoked) proofing of exactly that `(subject, person_id)`, so a wrong or stale link in the
//! identity provider's attributes reaches nothing.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const METHODS: [&str; 4] = ["in_person", "video_id_check", "nhs_login", "postal_code"];

#[derive(Debug, Clone, Serialize)]
pub struct Proofing {
    pub person_id: Uuid,
    pub method: String,
    pub proofed_by: String,
    pub proofed_at: DateTime<Utc>,
}

pub struct IdentityService {
    db: DatabaseConnection,
}

impl IdentityService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    fn subject(raw: &str) -> Result<String> {
        let s = raw.trim();
        if s.is_empty() || s.len() > 200 || s.chars().any(char::is_control) {
            return Err(Error::validation(
                "subject must be 1 to 200 characters with no control characters",
            ));
        }
        Ok(s.to_string())
    }

    /// Record that staff checked the account `subject` is this patient. A live proofing of the same account
    /// is superseded (revoked, kept in history) and replaced.
    pub async fn record(
        &self,
        subject: &str,
        person_id: Uuid,
        method: &str,
        proofed_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let subject = Self::subject(subject)?;
        if !METHODS.contains(&method) {
            return Err(Error::validation(format!(
                "method must be one of {}",
                METHODS.join(", ")
            )));
        }
        let by = crate::validation::named_person(
            proofed_by,
            "proofing is recorded by a named person (proofed_by)",
        )?
        .to_string();
        let (method, ctx) = (method.to_string(), ctx.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE identity_proofings SET revoked_at = now(), revoked_by = $2 \
                         WHERE subject = $1 AND revoked_at IS NULL",
                        [subject.clone().into(), format!("superseded by {by}").into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO identity_proofings (id, subject, person_id, method, proofed_by) VALUES ($1,$2,$3,$4,$5)",
                        [
                            Uuid::new_v4().into(),
                            subject.into(),
                            person_id.into(),
                            method.clone().into(),
                            by.clone().into(),
                        ],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    AuditLogRepository::log_with(
                        txn,
                        "patient",
                        person_id,
                        "identity_proofed",
                        None,
                        Some(serde_json::json!({ "method": method, "proofed_by": by })),
                        Some(&method),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)
    }

    /// Withdraw the proofing of an account: the portal refuses it from now on.
    pub async fn revoke(&self, subject: &str, revoked_by: &str, ctx: &UserContext) -> Result<()> {
        let subject = Self::subject(subject)?;
        let by = crate::validation::named_person(
            revoked_by,
            "revoking is done by a named person (revoked_by)",
        )?;
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE identity_proofings SET revoked_at = now(), revoked_by = $2 \
                 WHERE subject = $1 AND revoked_at IS NULL RETURNING person_id",
                [subject.into(), by.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found("no live identity proofing for that account"))?;
        let person_id: Uuid = row.try_get("", "person_id").map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person_id,
            "identity_proofing_revoked",
            None,
            Some(serde_json::json!({ "revoked_by": by })),
            ctx,
        )
        .await
    }

    /// Is this account proofed as exactly this patient?
    pub async fn check(&self, subject: &str, person_id: Uuid) -> Result<bool> {
        let subject = Self::subject(subject)?;
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM identity_proofings WHERE subject = $1 AND person_id = $2 AND revoked_at IS NULL",
                [subject.into(), person_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .is_some())
    }

    /// The live proofing of an account, for staff.
    pub async fn get(&self, subject: &str) -> Result<Option<Proofing>> {
        let subject = Self::subject(subject)?;
        self.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id, method, proofed_by, proofed_at FROM identity_proofings \
                 WHERE subject = $1 AND revoked_at IS NULL",
                [subject.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| {
                Ok(Proofing {
                    person_id: r.try_get("", "person_id").map_err(Error::Database)?,
                    method: r.try_get("", "method").map_err(Error::Database)?,
                    proofed_by: r.try_get("", "proofed_by").map_err(Error::Database)?,
                    proofed_at: r.try_get("", "proofed_at").map_err(Error::Database)?,
                })
            })
            .transpose()
    }
}

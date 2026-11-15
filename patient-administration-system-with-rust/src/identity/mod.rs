//! Identity proofing (DTAC-9).
//!
//! The patient portal serves a signed-in account only after staff have recorded that they checked who the
//! account belongs to, and which patient it is. [`IdentityService::check`] answers one question, "is this
//! account proofed as this patient?", and the portal asks it before every patient page. It answers yes only
//! for a live (unrevoked) proofing of exactly that `(subject, person_id)`, so a wrong or stale link in the
//! identity provider's attributes reaches nothing.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

/// How staff checked who an account belongs to. `digital_identity` is a national or regional digital identity
/// scheme (a government login, a bank identity, an eID card), named by the proofing's `provider`; no one
/// scheme is built in (QUAL-40, NFR-26).
pub const METHODS: [&str; 4] = [
    "in_person",
    "video_id_check",
    "digital_identity",
    "postal_code",
];

/// The method whose proofing names its provider.
pub const DIGITAL_IDENTITY: &str = "digital_identity";

/// The longest provider name accepted.
pub const MAX_PROVIDER: usize = 100;

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Proofing {
    pub person_id: Uuid,
    pub method: String,
    /// The digital identity scheme, for `digital_identity`; otherwise absent.
    #[schema(required = true)]
    pub provider: Option<String>,
    pub proofed_by: String,
    pub proofed_at: DateTime<Utc>,
}

pub struct IdentityService {
    db: Pool,
}

impl IdentityService {
    pub fn new(db: Pool) -> Self {
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

    /// A `digital_identity` proofing names its scheme; any other method names none.
    fn provider(method: &str, provider: Option<&str>) -> Result<Option<String>> {
        let provider = provider.map(str::trim).filter(|p| !p.is_empty());
        match (method == DIGITAL_IDENTITY, provider) {
            (true, None) => Err(Error::validation(
                "a digital_identity proofing names its provider (the identity scheme used)",
            )),
            (true, Some(p))
                if p.chars().count() > MAX_PROVIDER || p.chars().any(char::is_control) =>
            {
                Err(Error::validation(format!(
                    "provider must be at most {MAX_PROVIDER} characters with no control characters"
                )))
            }
            (true, Some(p)) => Ok(Some(p.to_string())),
            (false, Some(_)) => Err(Error::validation(
                "only a digital_identity proofing names a provider",
            )),
            (false, None) => Ok(None),
        }
    }

    /// Record that staff checked the account `subject` is this patient. A live proofing of the same account
    /// is superseded (revoked, kept in history) and replaced.
    pub async fn record(
        &self,
        subject: &str,
        person_id: Uuid,
        method: &str,
        provider: Option<&str>,
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
        let provider = Self::provider(method, provider)?;
        let by = crate::validation::named_person(
            proofed_by,
            "proofing is recorded by a named person (proofed_by)",
        )?
        .to_string();
        let (method, ctx) = (method.to_string(), ctx.clone());
        self.db
            .in_txn::<_, ()>(|txn| {
                Box::pin(async move {
                    txn.run(Stmt::new("UPDATE identity_proofings SET revoked_at = now(), revoked_by = $2 \
                         WHERE subject = $1 AND revoked_at IS NULL",
                        [subject.clone().into(), format!("superseded by {by}").into()],
                    ))
.await?;
                    txn.run(Stmt::new("INSERT INTO identity_proofings (id, subject, person_id, method, provider, proofed_by) VALUES ($1,$2,$3,$4,$5,$6)",
                        [
                            Uuid::new_v4().into(),
                            subject.into(),
                            person_id.into(),
                            method.clone().into(),
                            provider.clone().into(),
                            by.clone().into(),
                        ],
                    ))
.await?;
                    AuditLogRepository::log_with(
                        txn,
                        "patient",
                        person_id,
                        "identity_proofed",
                        None,
                        Some(serde_json::json!({ "method": method, "provider": provider, "proofed_by": by })),
                        Some(&method),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
.await
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
            .one(Stmt::new(
                "UPDATE identity_proofings SET revoked_at = now(), revoked_by = $2 \
                 WHERE subject = $1 AND revoked_at IS NULL RETURNING person_id",
                [subject.into(), by.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found("no live identity proofing for that account"))?;
        let person_id: Uuid = row.col("person_id")?;
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
            .one(Stmt::new("SELECT 1 AS ok FROM identity_proofings WHERE subject = $1 AND person_id = $2 AND revoked_at IS NULL",
                [subject.into(), person_id.into()],
            ))
.await?
            .is_some())
    }

    /// The live proofing of an account, for staff.
    pub async fn get(&self, subject: &str) -> Result<Option<Proofing>> {
        let subject = Self::subject(subject)?;
        self.db
            .one(Stmt::new(
                "SELECT person_id, method, provider, proofed_by, proofed_at FROM identity_proofings \
                 WHERE subject = $1 AND revoked_at IS NULL",
                [subject.into()],
            ))
            .await?
            .map(|r| {
                Ok(Proofing {
                    person_id: r.col("person_id")?,
                    method: r.col("method")?,
                    provider: r.col("provider")?,
                    proofed_by: r.col("proofed_by")?,
                    proofed_at: r.col("proofed_at")?,
                })
            })
            .transpose()
    }
}

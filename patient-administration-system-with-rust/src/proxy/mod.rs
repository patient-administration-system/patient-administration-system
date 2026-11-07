//! Carer and proxy authority (NEW-7).
//!
//! A named person records that another person (the proxy) may act for a patient: which actions (the scopes
//! `receive`, `confirm`, `cancel`, `rebook`), on what basis, until when, and who recorded it. A proxy with
//! no scope for an action cannot take it, an expired or revoked authority does nothing, and every action
//! taken through one is audited as the proxy acting for the patient. The PAS trusts its caller (the portal)
//! to have authenticated the proxy; it decides only what that proxy may do for that patient.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Row, Stmt, Transact};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

pub const SCOPES: [&str; 4] = ["receive", "confirm", "cancel", "rebook"];
pub const BASES: [&str; 5] = [
    "patient_consent",
    "parental_responsibility",
    "lasting_power_of_attorney",
    "court_appointed",
    "other",
];
/// The longest an authority may run before it must be recorded again.
pub const MAX_DAYS: i64 = 1095;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Authority {
    pub id: Uuid,
    pub person_id: Uuid,
    pub proxy_person_id: Uuid,
    pub relationship: String,
    pub basis: String,
    pub scopes: Vec<String>,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[schema(required = true)]
    pub revoked_by: Option<String>,
    #[schema(required = true)]
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Authority {
    /// Live at `now`: not revoked and not past its expiry.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && now < self.expires_at
    }
    pub fn allows(&self, scope: &str, now: DateTime<Utc>) -> bool {
        self.is_live(now) && self.scopes.iter().any(|s| s == scope)
    }
}

/// What a person supplies to record an authority.
#[derive(Debug, Clone)]
pub struct NewAuthority<'a> {
    pub proxy_person_id: Uuid,
    pub relationship: &'a str,
    pub basis: &'a str,
    pub scopes: &'a [String],
    pub expires_at: DateTime<Utc>,
    pub recorded_by: &'a str,
}

/// The audit context for an action a proxy takes for a patient: the proxy is the actor.
pub fn proxy_context(base: &UserContext, authority: &Authority) -> UserContext {
    UserContext {
        user_id: Some(format!(
            "proxy:{} for patient:{} (authority {})",
            authority.proxy_person_id, authority.person_id, authority.id
        )),
        user_ip: base.user_ip.clone(),
        user_agent: base.user_agent.clone(),
    }
}

const COLS: &str = "id, person_id, proxy_person_id, relationship, basis, scopes, recorded_by, recorded_at, \
    expires_at, revoked_by, revoked_at";

fn from_row(r: &Row) -> Result<Authority> {
    Ok(Authority {
        id: r.col("id")?,
        person_id: r.col("person_id")?,
        proxy_person_id: r.col("proxy_person_id")?,
        relationship: r.col("relationship")?,
        basis: r.col("basis")?,
        scopes: r.col("scopes")?,
        recorded_by: r.col("recorded_by")?,
        recorded_at: r.col("recorded_at")?,
        expires_at: r.col("expires_at")?,
        revoked_by: r.col("revoked_by")?,
        revoked_at: r.col("revoked_at")?,
    })
}

pub struct ProxyService {
    db: Pool,
}

impl ProxyService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// Record an authority. A named person; audited.
    pub async fn grant(
        &self,
        person_id: Uuid,
        new: &NewAuthority<'_>,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Authority> {
        let by = named_person(
            new.recorded_by,
            "recorded_by: the person recording the authority is required",
        )?;
        if new.proxy_person_id == person_id {
            return Err(Error::validation("a patient cannot be their own proxy"));
        }
        let relationship = new.relationship.trim();
        if relationship.is_empty()
            || relationship.chars().count() > 60
            || relationship.chars().any(char::is_control)
        {
            return Err(Error::validation(
                "relationship is required (at most 60 characters)",
            ));
        }
        if !BASES.contains(&new.basis) {
            return Err(Error::validation(format!(
                "basis must be one of {}",
                BASES.join(", ")
            )));
        }
        let mut scopes: Vec<String> = new.scopes.to_vec();
        scopes.sort();
        scopes.dedup();
        if scopes.is_empty() || scopes.iter().any(|s| !SCOPES.contains(&s.as_str())) {
            return Err(Error::validation(format!(
                "scopes must be a non-empty subset of {}",
                SCOPES.join(", ")
            )));
        }
        if new.expires_at <= now || new.expires_at > now + Duration::days(MAX_DAYS) {
            return Err(Error::validation(format!(
                "expires_at must be in the future and within {MAX_DAYS} days"
            )));
        }
        let id = Uuid::new_v4();
        let txn = self.db.begin_txn().await?;
        let row = txn
            .one(Stmt::new(format!(
                    "INSERT INTO proxy_authorities (id, person_id, proxy_person_id, relationship, basis, scopes, recorded_by, recorded_at, expires_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING {COLS}"
                ),
                [
                    id.into(),
                    person_id.into(),
                    new.proxy_person_id.into(),
                    relationship.into(),
                    new.basis.into(),
                    scopes.clone().into(),
                    by.into(),
                    now.into(),
                    new.expires_at.into(),
                ],
            ))
.await?
            .ok_or_else(|| Error::internal("the authority was not stored"))?;
        AuditLogRepository::log(
            &txn,
            "proxy_authority",
            id,
            "grant",
            None,
            Some(serde_json::json!({
                "person_id": person_id, "proxy_person_id": new.proxy_person_id,
                "basis": new.basis, "scopes": scopes, "recorded_by": by, "expires_at": new.expires_at,
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        from_row(&row)
    }

    /// Withdraw an authority, once. A named person; audited.
    pub async fn revoke(
        &self,
        id: Uuid,
        revoked_by: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            revoked_by,
            "revoked_by: the person withdrawing the authority is required",
        )?;
        let txn = self.db.begin_txn().await?;
        let done = txn
            .run(Stmt::new("UPDATE proxy_authorities SET revoked_by = $2, revoked_at = $3 WHERE id = $1 AND revoked_at IS NULL",
                [id.into(), by.into(), now.into()],
            ))
.await?;
        if done.rows_affected() != 1 {
            return Err(Error::not_found(format!("unrevoked proxy authority {id}")));
        }
        AuditLogRepository::log(
            &txn,
            "proxy_authority",
            id,
            "revoke",
            None,
            Some(serde_json::json!({ "revoked_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// A patient's authorities, newest first; live ones only unless `include_inactive`.
    pub async fn for_patient(
        &self,
        person_id: Uuid,
        include_inactive: bool,
        now: DateTime<Utc>,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<Authority>> {
        let rows = rows.into();
        let rows = self
            .db
            .all(Stmt::new(
                format!(
                    "SELECT {COLS} FROM proxy_authorities WHERE person_id = $1 \
                       AND ($2 OR (revoked_at IS NULL AND expires_at > $3)) \
                     ORDER BY recorded_at DESC, id LIMIT $4 OFFSET $5"
                ),
                [
                    person_id.into(),
                    include_inactive.into(),
                    now.into(),
                    rows.fetch().into(),
                    rows.offset().into(),
                ],
            ))
            .await?;
        rows.iter().map(from_row).collect()
    }

    /// The authority that lets `proxy_person_id` take `scope` for `person_id` at `now`, or `Forbidden`.
    /// The refusal does not say whether an authority exists, expired or lacks the scope.
    pub async fn authorise(
        &self,
        person_id: Uuid,
        proxy_person_id: Uuid,
        scope: &str,
        now: DateTime<Utc>,
    ) -> Result<Authority> {
        if !SCOPES.contains(&scope) {
            return Err(Error::internal(format!("unknown proxy scope {scope:?}")));
        }
        let rows = self
            .db
            .all(Stmt::new(format!(
                    "SELECT {COLS} FROM proxy_authorities WHERE person_id = $1 AND proxy_person_id = $2 \
                       AND revoked_at IS NULL AND expires_at > $3 AND $4 = ANY(scopes) \
                     ORDER BY expires_at DESC LIMIT 1"
                ),
                [
                    person_id.into(),
                    proxy_person_id.into(),
                    now.into(),
                    scope.into(),
                ],
            ))
.await?;
        match rows.first() {
            Some(r) => from_row(r),
            None => Err(Error::forbidden(
                "no current authority to act for this patient in this way",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(scopes: &[&str], expires_in: i64, revoked: bool) -> Authority {
        let now = Utc::now();
        Authority {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            proxy_person_id: Uuid::new_v4(),
            relationship: "daughter".into(),
            basis: "patient_consent".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            recorded_by: "Receptionist".into(),
            recorded_at: now,
            expires_at: now + Duration::days(expires_in),
            revoked_by: revoked.then(|| "Receptionist".to_string()),
            revoked_at: revoked.then_some(now),
        }
    }

    #[test]
    fn a_scope_is_allowed_only_while_live_and_listed() {
        let now = Utc::now();
        assert!(authority(&["receive", "cancel"], 10, false).allows("cancel", now));
        assert!(!authority(&["receive"], 10, false).allows("cancel", now));
        assert!(!authority(&["cancel"], -1, false).allows("cancel", now));
        assert!(!authority(&["cancel"], 10, true).allows("cancel", now));
    }

    #[test]
    fn the_audit_actor_names_the_proxy_and_the_patient() {
        let a = authority(&["cancel"], 10, false);
        let c = proxy_context(&UserContext::system(), &a);
        let who = c.user_id.unwrap();
        assert!(who.contains(&a.proxy_person_id.to_string()));
        assert!(who.contains(&a.person_id.to_string()));
        assert!(who.starts_with("proxy:"));
    }
}

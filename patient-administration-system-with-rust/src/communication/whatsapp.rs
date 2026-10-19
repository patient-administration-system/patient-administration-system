//! WhatsApp rules (CAP-13): an opt-in and an approved template.
//!
//! A WhatsApp message goes only to a patient who has opted in on WhatsApp (recorded with who recorded it and
//! how), and only using a template the provider has approved (recorded with the provider's template id). Both
//! are checked where every message is generated, when `PAS_WHATSAPP_REQUIRE_OPTIN=true`. That switch is off
//! by default only because no real WhatsApp carrier is connected yet (NOT-3); a production deployment must set
//! it before WhatsApp goes live (`operations/deployment.md`). STOP still silences WhatsApp whatever is
//! recorded here (`consent`).

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const SOURCES: [&str; 4] = ["patient_reply", "portal", "in_person", "paper"];

pub fn required() -> bool {
    std::env::var("PAS_WHATSAPP_REQUIRE_OPTIN").as_deref() == Ok("true")
}

/// Refuse (as a conflict, so the ladder tries the next channel) unless the patient has opted in on WhatsApp
/// and the template is approved. Does nothing unless the rule is switched on.
pub async fn check<C: ConnectionTrait>(conn: &C, person_id: Uuid, template_id: Uuid) -> Result<()> {
    if !required() {
        return Ok(());
    }
    let opted = conn
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 AS ok FROM whatsapp_optins WHERE person_id = $1 AND withdrawn_at IS NULL",
            [person_id.into()],
        ))
        .await
        .map_err(Error::Database)?
        .is_some();
    if !opted {
        return Err(Error::conflict(
            "the recipient has not opted in to WhatsApp messages",
        ));
    }
    let approved = conn
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 AS ok FROM whatsapp_template_approvals WHERE template_id = $1 AND revoked_at IS NULL",
            [template_id.into()],
        ))
        .await
        .map_err(Error::Database)?
        .is_some();
    if !approved {
        return Err(Error::conflict(
            "that template has not been approved for WhatsApp by the provider",
        ));
    }
    Ok(())
}

pub struct WhatsAppService {
    db: DatabaseConnection,
}

impl WhatsAppService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn record_optin(
        &self,
        person: Uuid,
        source: &str,
        recorded_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !SOURCES.contains(&source) {
            return Err(Error::validation(format!(
                "source must be one of {}",
                SOURCES.join(", ")
            )));
        }
        let by = recorded_by.trim();
        if by.is_empty() {
            return Err(Error::validation(
                "an opt-in is recorded by a named person (recorded_by)",
            ));
        }
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO whatsapp_optins (person_id, source, recorded_by) VALUES ($1,$2,$3) \
                 ON CONFLICT (person_id) DO UPDATE SET source = $2, recorded_by = $3, opted_in_at = now(), withdrawn_at = NULL",
                [person.into(), source.into(), by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person,
            "whatsapp_optin_recorded",
            None,
            Some(serde_json::json!({ "source": source, "recorded_by": by })),
            ctx,
        )
        .await
    }

    pub async fn withdraw_optin(&self, person: Uuid, ctx: &UserContext) -> Result<()> {
        let r = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE whatsapp_optins SET withdrawn_at = now() WHERE person_id = $1 AND withdrawn_at IS NULL",
                [person.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::not_found("no live WhatsApp opt-in for that patient"));
        }
        AuditLogRepository::log(
            &self.db,
            "patient",
            person,
            "whatsapp_optin_withdrawn",
            None,
            None,
            ctx,
        )
        .await
    }

    pub async fn approve_template(
        &self,
        template: Uuid,
        provider_template_id: &str,
        approved_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let (pid, by) = (provider_template_id.trim(), approved_by.trim());
        if pid.is_empty() || pid.len() > 200 || by.is_empty() {
            return Err(Error::validation(
                "a template approval needs the provider's template id and a named person (approved_by)",
            ));
        }
        let exists = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM letter_templates WHERE id = $1",
                [template.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if exists.is_none() {
            return Err(Error::not_found(format!("letter template {template}")));
        }
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO whatsapp_template_approvals (template_id, provider_template_id, approved_by) VALUES ($1,$2,$3) \
                 ON CONFLICT (template_id) DO UPDATE SET provider_template_id = $2, approved_by = $3, approved_at = now(), revoked_at = NULL",
                [template.into(), pid.into(), by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "letter_template",
            template,
            "whatsapp_approved",
            None,
            Some(serde_json::json!({ "provider_template_id": pid, "approved_by": by })),
            ctx,
        )
        .await
    }

    pub async fn revoke_template(&self, template: Uuid, ctx: &UserContext) -> Result<()> {
        let r = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE whatsapp_template_approvals SET revoked_at = now() WHERE template_id = $1 AND revoked_at IS NULL",
                [template.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::not_found(
                "no live WhatsApp approval for that template",
            ));
        }
        AuditLogRepository::log(
            &self.db,
            "letter_template",
            template,
            "whatsapp_approval_revoked",
            None,
            None,
            ctx,
        )
        .await
    }
}

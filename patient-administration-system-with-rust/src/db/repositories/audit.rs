//! audit repository — writes rows to `audit_log` within the same DB
//! transaction as the entity change.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::Result;
use crate::db::entities::audit_log;

/// Per-request context attached to audit rows. Populated from request
/// headers (`X-User-Id`, `X-User-Ip`, `X-User-Agent`) by the HTTP layer.
///
/// All fields are optional; the audit trail still records the action even
/// when the caller is anonymous.
#[derive(Debug, Clone, Default)]
pub struct UserContext {
    pub user_id: Option<String>,
    pub user_ip: Option<String>,
    pub user_agent: Option<String>,
}

impl UserContext {
    /// Build a UserContext with no caller details. Useful for system
    /// background tasks (e.g. outbox dispatcher).
    pub fn system() -> Self {
        Self {
            user_id: Some("system".to_string()),
            user_ip: None,
            user_agent: None,
        }
    }
}

pub struct AuditLogRepository;

impl AuditLogRepository {
    /// Insert one audit row.
    ///
    /// Pass `old_value = None` for creates, `new_value = None` for deletes,
    /// and both for updates.
    pub async fn log<C: Conn>(
        conn: &C,
        entity_type: &str,
        entity_id: Uuid,
        action: &str,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        ctx: &UserContext,
    ) -> Result<()> {
        Self::log_with(
            conn,
            entity_type,
            entity_id,
            action,
            old_value,
            new_value,
            None,
            None,
            ctx,
        )
        .await
    }

    /// [`Self::log`] with the reason for the change as a short code, and the facts around it (CAP-6).
    #[allow(
        clippy::too_many_arguments,
        reason = "one audit row's columns; a struct would only rename them"
    )]
    pub async fn log_with<C: Conn>(
        conn: &C,
        entity_type: &str,
        entity_id: Uuid,
        action: &str,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        reason_code: Option<&str>,
        context: Option<serde_json::Value>,
        ctx: &UserContext,
    ) -> Result<()> {
        let am = audit_log::Model {
            id: Uuid::new_v4(),
            entity_type: entity_type.to_string(),
            entity_id,
            action: action.to_string(),
            old_value,
            new_value,
            user_id: ctx.user_id.clone(),
            user_ip: ctx.user_ip.clone(),
            user_agent: ctx.user_agent.clone(),
            reason_code: reason_code.map(String::from),
            context,
            at: chrono::Utc::now().fixed_offset(),
        };
        am.insert(conn).await?;
        Ok(())
    }

    /// Recent audit rows across all entities, newest first.
    pub async fn list_recent<C: Conn>(conn: &C, limit: u64) -> Result<Vec<audit_log::Model>> {
        conn.all_as::<audit_log::Model>(Stmt::new(
            "SELECT * FROM audit_log ORDER BY at DESC LIMIT $1",
            [(limit as i64).into()],
        ))
        .await
    }

    /// Audit rows for a single entity, newest first.
    pub async fn list_for_entity<C: Conn>(
        conn: &C,
        entity_type: &str,
        entity_id: Uuid,
        limit: u64,
    ) -> Result<Vec<audit_log::Model>> {
        conn.all_as::<audit_log::Model>(Stmt::new("SELECT * FROM audit_log WHERE entity_type = $1 AND entity_id = $2 ORDER BY at DESC LIMIT $3", [entity_type.into(), entity_id.into(), (limit as i64).into()])).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_context_default_is_empty() {
        let c = UserContext::default();
        assert!(c.user_id.is_none());
        assert!(c.user_ip.is_none());
        assert!(c.user_agent.is_none());
    }

    #[test]
    fn test_user_context_system_marks_system_user() {
        let c = UserContext::system();
        assert_eq!(c.user_id.as_deref(), Some("system"));
        assert!(c.user_ip.is_none());
        assert!(c.user_agent.is_none());
    }
}

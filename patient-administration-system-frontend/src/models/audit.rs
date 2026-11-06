//! `audit_log` table — read-only.

use sqlx::PgPool;

#[derive(Debug, sqlx::FromRow)]
pub struct AuditSummary {
    pub entity_type: String,
    pub action: String,
    pub user_id: Option<String>,
    pub at: chrono::DateTime<chrono::FixedOffset>,
}

pub async fn list_recent_audit(db: &PgPool, limit: i64) -> Result<Vec<AuditSummary>, sqlx::Error> {
    sqlx::query_as(
        "SELECT entity_type, action, user_id, at FROM audit_log ORDER BY at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

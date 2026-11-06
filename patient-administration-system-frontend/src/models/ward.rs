//! `wards` table — read-only.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize, Debug, Clone, sqlx::FromRow)]
pub struct WardSummary {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}

pub async fn list_active_wards(db: &PgPool) -> Result<Vec<WardSummary>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, code FROM wards WHERE active = true")
        .fetch_all(db)
        .await
}

/// Batch-load wards by id. Used to attach ward names to bed-picker rows.
pub async fn find_wards_by_ids(db: &PgPool, ids: &[Uuid]) -> Result<Vec<WardSummary>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT id, name, code FROM wards WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(db)
        .await
}

/// Load one ward by its primary-key id.
pub async fn find_ward_by_id(db: &PgPool, id: Uuid) -> Result<Option<WardSummary>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, code FROM wards WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

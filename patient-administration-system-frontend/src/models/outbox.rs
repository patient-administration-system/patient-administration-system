//! `outbox_events` table — read-only count for the dashboard.

use sqlx::PgPool;

pub async fn count_unpublished(db: &PgPool) -> Result<usize, sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE published = false")
        .fetch_one(db)
        .await?;
    Ok(n as usize)
}

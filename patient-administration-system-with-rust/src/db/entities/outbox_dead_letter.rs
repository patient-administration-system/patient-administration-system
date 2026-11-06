//! The row of the `outbox_dead_letters` table (v0.5.0).
//!
//! Holds outbox rows that exceeded `PAS_OUTBOX_MAX_RETRIES` failed publish
//! attempts. The dispatcher moves a row here in one transaction (insert
//! dead-letter + delete original), so an operator can review and optionally
//! replay it via `POST /api/admin/outbox/dead-letters/{id}/replay`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = outbox::DeadLetter)]
pub struct Model {
    /// New UUID for the dead-letter row. Not the same as `original_id`.
    pub id: Uuid,
    /// The `id` the row carried while it was in `outbox_events`. Preserved
    /// so an operator can correlate against application logs that recorded
    /// the original outbox id.
    pub original_id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    /// Original `outbox_events.at` — the moment the producing transaction
    /// committed the event.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    /// Moment the row was moved into the DLQ. Index-ordered.
    #[schema(value_type = String, format = DateTime)]
    pub dead_lettered_at: chrono::DateTime<chrono::FixedOffset>,
    /// Final retry_count when the move happened — equal to the configured
    /// `PAS_OUTBOX_MAX_RETRIES` at that time.
    pub retry_count: i32,
    /// Last error the publisher returned before the move. Never empty.
    pub last_error: String,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            original_id: row.col("original_id")?,
            event_type: row.col("event_type")?,
            payload: row.col("payload")?,
            created_at: row.col("created_at")?,
            dead_lettered_at: row.col("dead_lettered_at")?,
            retry_count: row.col("retry_count")?,
            last_error: row.col("last_error")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "outbox_dead_letters";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "original_id",
        "event_type",
        "payload",
        "created_at",
        "dead_lettered_at",
        "retry_count",
        "last_error",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.original_id.into(),
            self.event_type.clone().into(),
            self.payload.clone().into(),
            self.created_at.into(),
            self.dead_lettered_at.into(),
            self.retry_count.into(),
            self.last_error.clone().into(),
        ]
    }
}

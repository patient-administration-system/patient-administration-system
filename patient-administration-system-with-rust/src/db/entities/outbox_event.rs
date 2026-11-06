//! The row of the `outbox_events` table.
//!
//! Schema lives in two migrations:
//! - `m20260520_000001_init` — base columns (`id`, `event_type`, `payload`,
//!   `published`, `at`).
//! - `m20260525_000002_outbox_dlq` — retry-tracking columns (`retry_count`,
//!   `last_attempted_at`, `last_error`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = outbox::Event)]
pub struct Model {
    pub id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub published: bool,
    #[schema(value_type = String, format = DateTime)]
    pub at: chrono::DateTime<chrono::FixedOffset>,
    /// Count of publish attempts that failed. Reset to 0 when the row is
    /// replayed from the dead-letter queue. Compared against
    /// `PAS_OUTBOX_MAX_RETRIES` by the dispatcher.
    pub retry_count: i32,
    /// Timestamp of the most recent publish attempt (success or failure).
    /// `None` until the dispatcher first touches the row.
    #[schema(value_type = String, format = DateTime, required = true, nullable = true)]
    pub last_attempted_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// Diagnostic from the most recent failed publish attempt. Cleared when
    /// the row is replayed; never set when the most recent attempt
    /// succeeded.
    #[schema(required = true)]
    pub last_error: Option<String>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            event_type: row.col("event_type")?,
            payload: row.col("payload")?,
            published: row.col("published")?,
            at: row.col("at")?,
            retry_count: row.col("retry_count")?,
            last_attempted_at: row.col("last_attempted_at")?,
            last_error: row.col("last_error")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "outbox_events";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "event_type",
        "payload",
        "published",
        "at",
        "retry_count",
        "last_attempted_at",
        "last_error",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.event_type.clone().into(),
            self.payload.clone().into(),
            self.published.into(),
            self.at.into(),
            self.retry_count.into(),
            self.last_attempted_at.into(),
            self.last_error.clone().into(),
        ]
    }
}

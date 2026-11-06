//! The row of the `rtt_clock_events` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub pathway_id: Uuid,
    pub kind: String,
    pub reason: Option<String>,
    pub event_at: chrono::DateTime<chrono::FixedOffset>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            pathway_id: row.col("pathway_id")?,
            kind: row.col("kind")?,
            reason: row.col("reason")?,
            event_at: row.col("event_at")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "rtt_clock_events";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "pathway_id",
        "kind",
        "reason",
        "event_at",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.pathway_id.into(),
            self.kind.clone().into(),
            self.reason.clone().into(),
            self.event_at.into(),
            self.created_at.into(),
        ]
    }
}

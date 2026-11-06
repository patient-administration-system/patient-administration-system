//! The row of the `appointment_series` table (v0.9.0).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub worker_id: Option<Uuid>,
    pub service_type: String,
    pub start_datetime: chrono::DateTime<chrono::FixedOffset>,
    pub duration_minutes: i32,
    /// Encoded [`crate::models::appointment_series::RecurrenceRule`].
    /// Stored as a JSONB blob; never queried into directly — the
    /// service deserialises before use.
    pub rule: serde_json::Value,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            worker_id: row.col("worker_id")?,
            service_type: row.col("service_type")?,
            start_datetime: row.col("start_datetime")?,
            duration_minutes: row.col("duration_minutes")?,
            rule: row.col("rule")?,
            status: row.col("status")?,
            reason: row.col("reason")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "appointment_series";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "worker_id",
        "service_type",
        "start_datetime",
        "duration_minutes",
        "rule",
        "status",
        "reason",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.person_id.into(),
            self.worker_id.into(),
            self.service_type.clone().into(),
            self.start_datetime.into(),
            self.duration_minutes.into(),
            self.rule.clone().into(),
            self.status.clone().into(),
            self.reason.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

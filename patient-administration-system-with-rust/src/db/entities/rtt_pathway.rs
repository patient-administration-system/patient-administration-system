//! The row of the `rtt_pathways` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub breach_weeks: i32,
    pub status: String,
    pub started_at: chrono::DateTime<chrono::FixedOffset>,
    pub stopped_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            target_service: row.col("target_service")?,
            breach_weeks: row.col("breach_weeks")?,
            status: row.col("status")?,
            started_at: row.col("started_at")?,
            stopped_at: row.col("stopped_at")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "rtt_pathways";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "target_service",
        "breach_weeks",
        "status",
        "started_at",
        "stopped_at",
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
            self.target_service.clone().into(),
            self.breach_weeks.into(),
            self.status.clone().into(),
            self.started_at.into(),
            self.stopped_at.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

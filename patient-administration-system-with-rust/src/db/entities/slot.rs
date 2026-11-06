//! The row of the `slots` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub start_datetime: chrono::DateTime<chrono::FixedOffset>,
    pub end_datetime: chrono::DateTime<chrono::FixedOffset>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            schedule_id: row.col("schedule_id")?,
            start_datetime: row.col("start_datetime")?,
            end_datetime: row.col("end_datetime")?,
            status: row.col("status")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "slots";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "schedule_id",
        "start_datetime",
        "end_datetime",
        "status",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.schedule_id.into(),
            self.start_datetime.into(),
            self.end_datetime.into(),
            self.status.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

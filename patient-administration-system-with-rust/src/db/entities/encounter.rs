//! The row of the `encounters` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub class: String,
    pub status: String,
    pub period_start: chrono::DateTime<chrono::FixedOffset>,
    pub period_end: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub worker_id: Option<Uuid>,
    pub department_id: Option<Uuid>,
    pub reason: Option<String>,
    pub deleted_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            class: row.col("class")?,
            status: row.col("status")?,
            period_start: row.col("period_start")?,
            period_end: row.col("period_end")?,
            worker_id: row.col("worker_id")?,
            department_id: row.col("department_id")?,
            reason: row.col("reason")?,
            deleted_at: row.col("deleted_at")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "encounters";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "class",
        "status",
        "period_start",
        "period_end",
        "worker_id",
        "department_id",
        "reason",
        "deleted_at",
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
            self.class.clone().into(),
            self.status.clone().into(),
            self.period_start.into(),
            self.period_end.into(),
            self.worker_id.into(),
            self.department_id.into(),
            self.reason.clone().into(),
            self.deleted_at.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

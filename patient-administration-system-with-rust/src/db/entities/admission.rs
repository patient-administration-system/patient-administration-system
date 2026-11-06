//! The row of the `admissions` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub bed_id: Uuid,
    pub admitting_worker_id: Option<Uuid>,
    pub admitted_at: chrono::DateTime<chrono::FixedOffset>,
    pub reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            encounter_id: row.col("encounter_id")?,
            bed_id: row.col("bed_id")?,
            admitting_worker_id: row.col("admitting_worker_id")?,
            admitted_at: row.col("admitted_at")?,
            reason: row.col("reason")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "admissions";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "encounter_id",
        "bed_id",
        "admitting_worker_id",
        "admitted_at",
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
            self.encounter_id.into(),
            self.bed_id.into(),
            self.admitting_worker_id.into(),
            self.admitted_at.into(),
            self.reason.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

//! The row of the `bed_assignments` table.

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
    pub assigned_at: chrono::DateTime<chrono::FixedOffset>,
    pub released_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            encounter_id: row.col("encounter_id")?,
            bed_id: row.col("bed_id")?,
            assigned_at: row.col("assigned_at")?,
            released_at: row.col("released_at")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "bed_assignments";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "encounter_id",
        "bed_id",
        "assigned_at",
        "released_at",
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
            self.assigned_at.into(),
            self.released_at.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

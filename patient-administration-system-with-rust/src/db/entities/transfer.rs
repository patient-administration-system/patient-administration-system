//! The row of the `transfers` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub admission_id: Uuid,
    pub from_bed_id: Uuid,
    pub to_bed_id: Uuid,
    pub reason: Option<String>,
    pub transferred_at: chrono::DateTime<chrono::FixedOffset>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            admission_id: row.col("admission_id")?,
            from_bed_id: row.col("from_bed_id")?,
            to_bed_id: row.col("to_bed_id")?,
            reason: row.col("reason")?,
            transferred_at: row.col("transferred_at")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "transfers";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "admission_id",
        "from_bed_id",
        "to_bed_id",
        "reason",
        "transferred_at",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.admission_id.into(),
            self.from_bed_id.into(),
            self.to_bed_id.into(),
            self.reason.clone().into(),
            self.transferred_at.into(),
            self.created_at.into(),
        ]
    }
}

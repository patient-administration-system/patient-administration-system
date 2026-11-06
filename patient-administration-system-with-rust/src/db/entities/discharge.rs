//! The row of the `discharges` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub admission_id: Uuid,
    pub discharging_worker_id: Option<Uuid>,
    pub discharged_at: chrono::DateTime<chrono::FixedOffset>,
    pub disposition: Option<String>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            admission_id: row.col("admission_id")?,
            discharging_worker_id: row.col("discharging_worker_id")?,
            discharged_at: row.col("discharged_at")?,
            disposition: row.col("disposition")?,
            notes: row.col("notes")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "discharges";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "admission_id",
        "discharging_worker_id",
        "discharged_at",
        "disposition",
        "notes",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.admission_id.into(),
            self.discharging_worker_id.into(),
            self.discharged_at.into(),
            self.disposition.clone().into(),
            self.notes.clone().into(),
            self.created_at.into(),
        ]
    }
}

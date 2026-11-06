//! The row of the `schedules` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub owner_kind: String,
    pub owner_id: Uuid,
    pub service_type: String,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            owner_kind: row.col("owner_kind")?,
            owner_id: row.col("owner_id")?,
            service_type: row.col("service_type")?,
            active: row.col("active")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "schedules";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "owner_kind",
        "owner_id",
        "service_type",
        "active",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.owner_kind.clone().into(),
            self.owner_id.into(),
            self.service_type.clone().into(),
            self.active.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

//! The row of the `rooms` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub ward_id: Uuid,
    pub name: String,
    pub code: String,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            ward_id: row.col("ward_id")?,
            name: row.col("name")?,
            code: row.col("code")?,
            active: row.col("active")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "rooms";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "ward_id",
        "name",
        "code",
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
            self.ward_id.into(),
            self.name.clone().into(),
            self.code.clone().into(),
            self.active.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

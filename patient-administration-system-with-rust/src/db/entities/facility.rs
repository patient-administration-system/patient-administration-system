//! The row of the `facilities` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub address: serde_json::Value,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            name: row.col("name")?,
            code: row.col("code")?,
            address: row.col("address")?,
            active: row.col("active")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "facilities";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "name",
        "code",
        "address",
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
            self.name.clone().into(),
            self.code.clone().into(),
            self.address.clone().into(),
            self.active.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

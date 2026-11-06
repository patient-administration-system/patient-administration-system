//! The row of the `wards` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub name: String,
    pub code: String,
    pub capacity: i32,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            facility_id: row.col("facility_id")?,
            name: row.col("name")?,
            code: row.col("code")?,
            capacity: row.col("capacity")?,
            active: row.col("active")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "wards";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "facility_id",
        "name",
        "code",
        "capacity",
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
            self.facility_id.into(),
            self.name.clone().into(),
            self.code.clone().into(),
            self.capacity.into(),
            self.active.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

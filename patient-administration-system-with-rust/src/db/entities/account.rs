//! The row of the `accounts` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub status: String,
    pub currency: String,
    pub opened_at: chrono::DateTime<chrono::FixedOffset>,
    pub closed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            status: row.col("status")?,
            currency: row.col("currency")?,
            opened_at: row.col("opened_at")?,
            closed_at: row.col("closed_at")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "accounts";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "status",
        "currency",
        "opened_at",
        "closed_at",
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
            self.status.clone().into(),
            self.currency.clone().into(),
            self.opened_at.into(),
            self.closed_at.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

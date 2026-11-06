//! The row of the `invoices` table.
//!
//! Note: `total_value` is stored as `String` at the DB layer. Conversion
//! to/from `crate::models::Money` happens in repositories.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub account_id: Uuid,
    pub status: String,
    pub total_value: String,
    pub total_currency: String,
    pub charge_ids: serde_json::Value,
    pub finalized_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            account_id: row.col("account_id")?,
            status: row.col("status")?,
            total_value: row.col("total_value")?,
            total_currency: row.col("total_currency")?,
            charge_ids: row.col("charge_ids")?,
            finalized_at: row.col("finalized_at")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "invoices";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "account_id",
        "status",
        "total_value",
        "total_currency",
        "charge_ids",
        "finalized_at",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.account_id.into(),
            self.status.clone().into(),
            self.total_value.clone().into(),
            self.total_currency.clone().into(),
            self.charge_ids.clone().into(),
            self.finalized_at.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

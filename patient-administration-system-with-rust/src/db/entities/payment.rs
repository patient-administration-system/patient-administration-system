//! The row of the `payments` table.
//!
//! Note: `amount_value` is stored as `String` at the DB layer. Conversion
//! to/from `crate::models::Money` happens in repositories.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub amount_value: String,
    pub amount_currency: String,
    pub method: String,
    pub reference: Option<String>,
    pub posted_at: chrono::DateTime<chrono::FixedOffset>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            invoice_id: row.col("invoice_id")?,
            amount_value: row.col("amount_value")?,
            amount_currency: row.col("amount_currency")?,
            method: row.col("method")?,
            reference: row.col("reference")?,
            posted_at: row.col("posted_at")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "payments";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "invoice_id",
        "amount_value",
        "amount_currency",
        "method",
        "reference",
        "posted_at",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.invoice_id.into(),
            self.amount_value.clone().into(),
            self.amount_currency.clone().into(),
            self.method.clone().into(),
            self.reference.clone().into(),
            self.posted_at.into(),
            self.created_at.into(),
        ]
    }
}

//! The row of the `charges` table.
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
    pub account_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
    pub code: String,
    pub description: String,
    pub amount_value: String,
    pub amount_currency: String,
    pub posted_at: chrono::DateTime<chrono::FixedOffset>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            account_id: row.col("account_id")?,
            encounter_id: row.col("encounter_id")?,
            appointment_id: row.col("appointment_id")?,
            code: row.col("code")?,
            description: row.col("description")?,
            amount_value: row.col("amount_value")?,
            amount_currency: row.col("amount_currency")?,
            posted_at: row.col("posted_at")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "charges";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "account_id",
        "encounter_id",
        "appointment_id",
        "code",
        "description",
        "amount_value",
        "amount_currency",
        "posted_at",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.account_id.into(),
            self.encounter_id.into(),
            self.appointment_id.into(),
            self.code.clone().into(),
            self.description.clone().into(),
            self.amount_value.clone().into(),
            self.amount_currency.clone().into(),
            self.posted_at.into(),
            self.created_at.into(),
        ]
    }
}

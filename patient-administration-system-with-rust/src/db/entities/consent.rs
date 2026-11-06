//! The row of the `consents` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub consent_type: String,
    pub status: String,
    pub granted_date: chrono::NaiveDate,
    pub expiry_date: Option<chrono::NaiveDate>,
    pub revoked_date: Option<chrono::NaiveDate>,
    pub purpose: Option<String>,
    pub method: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            consent_type: row.col("consent_type")?,
            status: row.col("status")?,
            granted_date: row.col("granted_date")?,
            expiry_date: row.col("expiry_date")?,
            revoked_date: row.col("revoked_date")?,
            purpose: row.col("purpose")?,
            method: row.col("method")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "consents";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "consent_type",
        "status",
        "granted_date",
        "expiry_date",
        "revoked_date",
        "purpose",
        "method",
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
            self.consent_type.clone().into(),
            self.status.clone().into(),
            self.granted_date.into(),
            self.expiry_date.into(),
            self.revoked_date.into(),
            self.purpose.clone().into(),
            self.method.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

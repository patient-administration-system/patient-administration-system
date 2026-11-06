//! The row of the `waitlist_entries` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub referral_id: Option<Uuid>,
    pub person_id: Uuid,
    pub target_service: String,
    pub priority: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            referral_id: row.col("referral_id")?,
            person_id: row.col("person_id")?,
            target_service: row.col("target_service")?,
            priority: row.col("priority")?,
            status: row.col("status")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "waitlist_entries";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "referral_id",
        "person_id",
        "target_service",
        "priority",
        "status",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.referral_id.into(),
            self.person_id.into(),
            self.target_service.clone().into(),
            self.priority.clone().into(),
            self.status.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

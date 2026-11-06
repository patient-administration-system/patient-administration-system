//! The row of the `referrals` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub referring_worker_id: Option<Uuid>,
    pub target_service: String,
    pub reason: Option<String>,
    pub received_at: chrono::DateTime<chrono::FixedOffset>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            referring_worker_id: row.col("referring_worker_id")?,
            target_service: row.col("target_service")?,
            reason: row.col("reason")?,
            received_at: row.col("received_at")?,
            created_at: row.col("created_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "referrals";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "referring_worker_id",
        "target_service",
        "reason",
        "received_at",
        "created_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.person_id.into(),
            self.referring_worker_id.into(),
            self.target_service.clone().into(),
            self.reason.clone().into(),
            self.received_at.into(),
            self.created_at.into(),
        ]
    }
}

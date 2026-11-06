//! The row of the `coverages` table (v0.10.0).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub person_id: Uuid,
    pub account_id: Option<Uuid>,
    pub status: String,
    pub kind: String,
    pub subscriber_id: Option<Uuid>,
    pub payor_name: String,
    pub payor_identifier: Option<String>,
    pub policy_number: String,
    pub group_number: Option<String>,
    pub relationship: String,
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            person_id: row.col("person_id")?,
            account_id: row.col("account_id")?,
            status: row.col("status")?,
            kind: row.col("kind")?,
            subscriber_id: row.col("subscriber_id")?,
            payor_name: row.col("payor_name")?,
            payor_identifier: row.col("payor_identifier")?,
            policy_number: row.col("policy_number")?,
            group_number: row.col("group_number")?,
            relationship: row.col("relationship")?,
            start_date: row.col("start_date")?,
            end_date: row.col("end_date")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "coverages";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "person_id",
        "account_id",
        "status",
        "kind",
        "subscriber_id",
        "payor_name",
        "payor_identifier",
        "policy_number",
        "group_number",
        "relationship",
        "start_date",
        "end_date",
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
            self.account_id.into(),
            self.status.clone().into(),
            self.kind.clone().into(),
            self.subscriber_id.into(),
            self.payor_name.clone().into(),
            self.payor_identifier.clone().into(),
            self.policy_number.clone().into(),
            self.group_number.clone().into(),
            self.relationship.clone().into(),
            self.start_date.into(),
            self.end_date.into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

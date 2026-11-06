//! The row of the `audit_log` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = audit::Entry)]
pub struct Model {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub action: String,
    #[schema(required = true)]
    pub old_value: Option<serde_json::Value>,
    #[schema(required = true)]
    pub new_value: Option<serde_json::Value>,
    #[schema(required = true)]
    pub user_id: Option<String>,
    #[schema(required = true)]
    pub user_ip: Option<String>,
    #[schema(required = true)]
    pub user_agent: Option<String>,
    #[schema(required = true)]
    pub reason_code: Option<String>,
    #[schema(required = true)]
    pub context: Option<serde_json::Value>,
    #[schema(value_type = String, format = DateTime)]
    pub at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            entity_type: row.col("entity_type")?,
            entity_id: row.col("entity_id")?,
            action: row.col("action")?,
            old_value: row.col("old_value")?,
            new_value: row.col("new_value")?,
            user_id: row.col("user_id")?,
            user_ip: row.col("user_ip")?,
            user_agent: row.col("user_agent")?,
            reason_code: row.col("reason_code")?,
            context: row.col("context")?,
            at: row.col("at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "audit_log";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "entity_type",
        "entity_id",
        "action",
        "old_value",
        "new_value",
        "user_id",
        "user_ip",
        "user_agent",
        "reason_code",
        "context",
        "at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.entity_type.clone().into(),
            self.entity_id.into(),
            self.action.clone().into(),
            self.old_value.clone().into(),
            self.new_value.clone().into(),
            self.user_id.clone().into(),
            self.user_ip.clone().into(),
            self.user_agent.clone().into(),
            self.reason_code.clone().into(),
            self.context.clone().into(),
            self.at.into(),
        ]
    }
}

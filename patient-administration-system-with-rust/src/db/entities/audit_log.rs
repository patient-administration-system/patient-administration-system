//! SeaORM entity for the `audit_log` table.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, utoipa::ToSchema)]
#[sea_orm(table_name = "audit_log")]
#[schema(as = audit::Entry)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
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
    pub at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

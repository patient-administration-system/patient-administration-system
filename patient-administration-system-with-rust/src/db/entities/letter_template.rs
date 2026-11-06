//! The row of the `letter_templates` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub body_tera: String,
    pub required_variables: serde_json::Value,
    pub channels: serde_json::Value,
    pub active: bool,
    pub locale: String,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            name: row.col("name")?,
            subject: row.col("subject")?,
            body_tera: row.col("body_tera")?,
            required_variables: row.col("required_variables")?,
            channels: row.col("channels")?,
            active: row.col("active")?,
            locale: row.col("locale")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "letter_templates";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "name",
        "subject",
        "body_tera",
        "required_variables",
        "channels",
        "active",
        "locale",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.name.clone().into(),
            self.subject.clone().into(),
            self.body_tera.clone().into(),
            self.required_variables.clone().into(),
            self.channels.clone().into(),
            self.active.into(),
            self.locale.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

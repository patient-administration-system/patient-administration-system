//! The row of the `generated_letters` table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{FromRow, Param, Row, Table};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub template_id: Uuid,
    pub person_id: Uuid,
    pub appointment_id: Option<Uuid>,
    pub rendered_subject: String,
    pub rendered_body: String,
    pub channel: String,
    pub status: String,
    pub sent_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub locale: Option<String>,
    pub trigger: String,
    pub trigger_ref: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

impl FromRow for Model {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.col("id")?,
            template_id: row.col("template_id")?,
            person_id: row.col("person_id")?,
            appointment_id: row.col("appointment_id")?,
            rendered_subject: row.col("rendered_subject")?,
            rendered_body: row.col("rendered_body")?,
            channel: row.col("channel")?,
            status: row.col("status")?,
            sent_at: row.col("sent_at")?,
            locale: row.col("locale")?,
            trigger: row.col("trigger")?,
            trigger_ref: row.col("trigger_ref")?,
            created_at: row.col("created_at")?,
            updated_at: row.col("updated_at")?,
        })
    }
}

impl Table for Model {
    const NAME: &'static str = "generated_letters";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "template_id",
        "person_id",
        "appointment_id",
        "rendered_subject",
        "rendered_body",
        "channel",
        "status",
        "sent_at",
        "locale",
        "trigger",
        "trigger_ref",
        "created_at",
        "updated_at",
    ];

    fn id(&self) -> Uuid {
        self.id
    }

    fn values(&self) -> Vec<Param> {
        vec![
            self.id.into(),
            self.template_id.into(),
            self.person_id.into(),
            self.appointment_id.into(),
            self.rendered_subject.clone().into(),
            self.rendered_body.clone().into(),
            self.channel.clone().into(),
            self.status.clone().into(),
            self.sent_at.into(),
            self.locale.clone().into(),
            self.trigger.clone().into(),
            self.trigger_ref.clone().into(),
            self.created_at.into(),
            self.updated_at.into(),
        ]
    }
}

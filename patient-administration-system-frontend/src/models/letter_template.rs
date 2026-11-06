//! `letter_templates` table — read-only summary projection for the
//! `/letters/new` composer form.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct TemplateRecord {
    id: Uuid,
    name: String,
    subject: String,
    channels: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateRow {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub channels: Vec<String>,
}

pub async fn list_active_templates(db: &PgPool) -> Result<Vec<TemplateRow>, sqlx::Error> {
    let rows: Vec<TemplateRecord> = sqlx::query_as(
        "SELECT id, name, subject, channels FROM letter_templates WHERE active = true",
    )
    .fetch_all(db)
    .await?;
    let mut out: Vec<TemplateRow> = rows
        .into_iter()
        .map(|m| TemplateRow {
            id: m.id,
            name: m.name,
            subject: m.subject,
            channels: m
                .channels
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

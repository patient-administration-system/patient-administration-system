//! A triage category assigned by someone else, recorded against an emergency encounter (NEW-22).
//!
//! The Manchester Triage System (MTS) sorts a presentation into five categories, each with a target time to
//! first clinician contact. Its flowcharts and discriminators are licensed content and the work of a trained
//! nurse or a licensed tool (plan §11, NEW-25). **The PAS records the result and never computes it**: no
//! discriminator text, no flowchart logic. A re-triage adds a row; the history is kept.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The triage systems whose result can be recorded.
pub const SYSTEMS: [&str; 1] = ["mts"];

/// The five categories, most urgent first. The colour is a display label only.
pub const CATEGORIES: [&str; 5] = [
    "immediate",
    "very_urgent",
    "urgent",
    "standard",
    "non_urgent",
];

/// The display colour for a category.
pub fn colour(category: &str) -> Option<&'static str> {
    match category {
        "immediate" => Some("red"),
        "very_urgent" => Some("orange"),
        "urgent" => Some("yellow"),
        "standard" => Some("green"),
        "non_urgent" => Some("blue"),
        _ => None,
    }
}

const MAX_TEXT: usize = 100;

#[derive(Debug, Serialize)]
pub struct TriageAssessment {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub triage_system: String,
    pub system_version: Option<String>,
    pub category: String,
    pub colour: Option<&'static str>,
    pub flowchart: Option<String>,
    pub assessed_by: String,
    pub assessed_at: DateTime<Utc>,
}

/// Optional free text: trimmed, blank becomes none, bounded, no control characters.
fn optional_text(value: Option<&str>, field: &str) -> Result<Option<String>> {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > MAX_TEXT || v.chars().any(char::is_control) {
        return Err(Error::validation(format!(
            "{field} must be at most {MAX_TEXT} characters with no control characters"
        )));
    }
    Ok(Some(v.to_string()))
}

pub struct TriageService {
    db: DatabaseConnection,
}

impl TriageService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record an assessment for an emergency encounter that is not finished or cancelled. `assessed_at` is
    /// when the assessor decided, not later than `now`.
    #[allow(
        clippy::too_many_arguments,
        reason = "one field of an assessment per argument; a struct would only restate them"
    )]
    pub async fn record(
        &self,
        encounter_id: Uuid,
        triage_system: &str,
        system_version: Option<&str>,
        category: &str,
        flowchart: Option<&str>,
        assessed_by: &str,
        assessed_at: DateTime<Utc>,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<TriageAssessment> {
        if !SYSTEMS.contains(&triage_system) {
            return Err(Error::validation(format!(
                "triage_system must be one of {}",
                SYSTEMS.join(", ")
            )));
        }
        if !CATEGORIES.contains(&category) {
            return Err(Error::validation(format!(
                "category must be one of {}",
                CATEGORIES.join(", ")
            )));
        }
        if assessed_at > now {
            return Err(Error::validation("assessed_at cannot be in the future"));
        }
        let assessor = named_person(
            assessed_by,
            "assessed_by: the person who assigned the category is required",
        )?;
        let version = optional_text(system_version, "system_version")?;
        let flowchart = optional_text(flowchart, "flowchart")?;

        let txn = self.db.begin().await.map_err(Error::Database)?;
        let enc = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT class, status FROM encounters WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
                [encounter_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("encounter {encounter_id}")))?;
        let class: String = enc.try_get("", "class").map_err(Error::Database)?;
        let status: String = enc.try_get("", "status").map_err(Error::Database)?;
        if class != "emergency" {
            return Err(Error::validation(
                "a triage category can only be recorded on an emergency encounter",
            ));
        }
        if status == "finished" || status == "cancelled" {
            return Err(Error::validation(format!(
                "the encounter is {status}; a triage category cannot be added"
            )));
        }
        let id = Uuid::new_v4();
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO triage_assessments \
               (id, encounter_id, triage_system, system_version, category, flowchart, assessed_by, assessed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            [
                id.into(),
                encounter_id.into(),
                triage_system.into(),
                version.clone().into(),
                category.into(),
                flowchart.clone().into(),
                assessor.into(),
                assessed_at.into(),
            ],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "encounter",
            encounter_id,
            "record_triage",
            None,
            Some(serde_json::json!({
                "assessment_id": id, "triage_system": triage_system, "system_version": version,
                "category": category, "flowchart": flowchart, "assessed_by": assessor,
                "assessed_at": assessed_at,
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(TriageAssessment {
            id,
            encounter_id,
            triage_system: triage_system.to_string(),
            system_version: version,
            category: category.to_string(),
            colour: colour(category),
            flowchart,
            assessed_by: assessor.to_string(),
            assessed_at,
        })
    }

    /// Every assessment of an encounter, oldest first. The last is the current one.
    pub async fn history(&self, encounter_id: Uuid) -> Result<Vec<TriageAssessment>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, triage_system, system_version, category, flowchart, assessed_by, assessed_at \
                 FROM triage_assessments WHERE encounter_id = $1 ORDER BY assessed_at, created_at, id",
                [encounter_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                let category: String = r.try_get("", "category").map_err(Error::Database)?;
                Ok(TriageAssessment {
                    id: r.try_get("", "id").map_err(Error::Database)?,
                    encounter_id,
                    triage_system: r.try_get("", "triage_system").map_err(Error::Database)?,
                    system_version: r.try_get("", "system_version").map_err(Error::Database)?,
                    colour: colour(&category),
                    category,
                    flowchart: r.try_get("", "flowchart").map_err(Error::Database)?,
                    assessed_by: r.try_get("", "assessed_by").map_err(Error::Database)?,
                    assessed_at: r.try_get("", "assessed_at").map_err(Error::Database)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_a_colour_and_unknown_ones_none() {
        for c in CATEGORIES {
            assert!(colour(c).is_some(), "{c}");
        }
        assert_eq!(colour("red"), None);
    }

    #[test]
    fn optional_text_is_trimmed_bounded_and_clean() {
        assert_eq!(
            optional_text(Some("  3.8 "), "v").unwrap().as_deref(),
            Some("3.8")
        );
        assert_eq!(optional_text(Some("  "), "v").unwrap(), None);
        assert_eq!(optional_text(None, "v").unwrap(), None);
        assert!(optional_text(Some(&"x".repeat(101)), "v").is_err());
        assert!(optional_text(Some("a\nb"), "v").is_err());
    }
}

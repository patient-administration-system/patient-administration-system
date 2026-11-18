//! A triage category assigned by someone else, recorded against an emergency encounter (NEW-22).
//!
//! The Manchester Triage System (MTS) sorts a presentation into five categories, each with a target time to
//! first clinician contact. Its flowcharts and discriminators are licensed content and the work of a trained
//! nurse or a licensed tool (plan §11, NEW-25). **The PAS records the result and never computes it**: no
//! discriminator text, no flowchart logic. A re-triage adds a row; the history is kept.

// The categories, the assessment record, the interchange codes, the targets and the waits arithmetic are
// domain rules (QUAL-52).
pub use patient_administration_system_domain::triage::*;
pub use patient_administration_system_domain::triage::{interchange, targets};
pub mod waits;

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact};
use jiff::Timestamp;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

pub struct TriageService {
    db: Pool,
}

impl TriageService {
    pub fn new(db: Pool) -> Self {
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
        assessed_at: Timestamp,
        now: Timestamp,
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

        let txn = self.db.begin_txn().await?;
        let enc = txn
            .one(Stmt::new("SELECT class, status FROM encounters WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
                [encounter_id.into()],
            ))
.await?
            .ok_or_else(|| Error::not_found(format!("encounter {encounter_id}")))?;
        let class: String = enc.col("class")?;
        let status: String = enc.col("status")?;
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
        txn.run(Stmt::new("INSERT INTO triage_assessments \
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
.await?;
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

    /// Every assessment of an encounter as FHIR `Observation`s, oldest first (NEW-24).
    pub async fn observations(&self, encounter_id: Uuid) -> Result<Vec<serde_json::Value>> {
        let person: Uuid = self
            .db
            .one(Stmt::new(
                "SELECT person_id FROM encounters WHERE id = $1 AND deleted_at IS NULL",
                [encounter_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("encounter {encounter_id}")))?
            .col("person_id")?;
        Ok(self
            .history(encounter_id)
            .await?
            .iter()
            .map(|a| interchange::to_observation(a, person))
            .collect())
    }

    /// Every assessment of an encounter, oldest first. The last is the current one.
    pub async fn history(&self, encounter_id: Uuid) -> Result<Vec<TriageAssessment>> {
        let rows = self
            .db
            .all(Stmt::new("SELECT id, triage_system, system_version, category, flowchart, assessed_by, assessed_at \
                 FROM triage_assessments WHERE encounter_id = $1 ORDER BY assessed_at, created_at, id",
                [encounter_id.into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                let category: String = r.col("category")?;
                Ok(TriageAssessment {
                    id: r.col("id")?,
                    encounter_id,
                    triage_system: r.col("triage_system")?,
                    system_version: r.col("system_version")?,
                    colour: colour(&category),
                    category,
                    flowchart: r.col("flowchart")?,
                    assessed_by: r.col("assessed_by")?,
                    assessed_at: r.col("assessed_at")?,
                })
            })
            .collect()
    }
}

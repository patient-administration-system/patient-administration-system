//! Time from arrival to first clinician contact, against the triage target (NEW-23).
//!
//! Arrival is the emergency encounter's `period_start`; first contact is `encounters.first_contact_at`,
//! recorded once by a named person. The report and the queue **only describe**: the order of care is a
//! clinician's decision, so the queue is a display (the same stance as NFR-11), and an attendance with no
//! triage category is counted as excluded rather than given a guessed one. The category used is the latest
//! assessment recorded.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Stmt, Transact};
use jiff::Timestamp;
use uuid::Uuid;

use super::TriageService;
use super::targets::Targets;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};
pub use patient_administration_system_domain::triage::waits::*;

const LATEST_CATEGORY: &str = "(SELECT t.category FROM triage_assessments t WHERE t.encounter_id = e.id \
     ORDER BY t.assessed_at DESC, t.created_at DESC, t.id DESC LIMIT 1)";

impl TriageService {
    /// Record when a clinician first saw the patient. Once only, by a named person, not before arrival and
    /// not in the future; only on an emergency encounter that is not cancelled.
    pub async fn record_first_contact(
        &self,
        encounter_id: Uuid,
        seen_by: &str,
        seen_at: Timestamp,
        now: Timestamp,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            seen_by,
            "seen_by: the clinician who first saw the patient is required",
        )?;
        if by.chars().count() > super::MAX_TEXT || by.chars().any(char::is_control) {
            return Err(Error::validation(format!(
                "seen_by must be at most {} characters with no control characters",
                super::MAX_TEXT
            )));
        }
        if seen_at > now {
            return Err(Error::validation("seen_at cannot be in the future"));
        }
        let txn = self.db.begin_txn().await?;
        let enc = txn
            .one(Stmt::new(
                "SELECT class, status, period_start, first_contact_at FROM encounters \
                 WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
                [encounter_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("encounter {encounter_id}")))?;
        let class: String = enc.col("class")?;
        let status: String = enc.col("status")?;
        let arrived: Timestamp = enc.col("period_start")?;
        let already: Option<Timestamp> = enc.col("first_contact_at")?;
        if class != "emergency" {
            return Err(Error::validation(
                "first clinician contact is only recorded on an emergency encounter",
            ));
        }
        if status == "cancelled" {
            return Err(Error::validation(
                "the encounter is cancelled; first contact cannot be recorded",
            ));
        }
        if already.is_some() {
            return Err(Error::conflict(
                "first clinician contact is already recorded for this encounter",
            ));
        }
        if seen_at < arrived {
            return Err(Error::validation("seen_at cannot be before arrival"));
        }
        txn.run(Stmt::new("UPDATE encounters SET first_contact_at = $2, first_contact_by = $3, updated_at = now() WHERE id = $1",
            [encounter_id.into(), seen_at.into(), by.into()],
        ))
.await?;
        AuditLogRepository::log(
            &txn,
            "encounter",
            encounter_id,
            "record_first_contact",
            None,
            Some(serde_json::json!({
                "seen_by": by, "seen_at": seen_at, "arrived_at": arrived,
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(())
    }

    async fn attendances(
        &self,
        sql: &str,
        values: Vec<crate::db::sql::Param>,
    ) -> Result<Vec<Attendance>> {
        let rows = self.db.all(Stmt::new(sql, values)).await?;
        rows.iter()
            .map(|r| {
                Ok(Attendance {
                    encounter_id: r.col("id")?,
                    arrived_at: r.col("period_start")?,
                    first_contact_at: r.col("first_contact_at")?,
                    category: r.col("category")?,
                })
            })
            .collect()
    }

    /// Attendances that arrived in `[from, to)` against the targets. A window of more than 31 days or one
    /// that ends before it starts is refused.
    pub async fn waits_report(
        &self,
        from: Timestamp,
        to: Timestamp,
        targets: &Targets,
        now: Timestamp,
    ) -> Result<WaitsReport> {
        if to <= from {
            return Err(Error::validation("to must be after from"));
        }
        if to - from > crate::time::days(31) {
            return Err(Error::validation("the window can be at most 31 days"));
        }
        let sql = format!(
            "SELECT e.id, e.period_start, e.first_contact_at, {LATEST_CATEGORY} AS category \
             FROM encounters e WHERE e.class = 'emergency' AND e.status <> 'cancelled' \
               AND e.deleted_at IS NULL AND e.period_start >= $1 AND e.period_start < $2 \
             ORDER BY e.period_start, e.id"
        );
        let rows = self.attendances(&sql, vec![from.into(), to.into()]).await?;
        Ok(build_report(&rows, targets, from, to, now))
    }

    /// Everyone who has arrived and not yet been seen, in display order. The order of care is a clinician's.
    pub async fn queue(&self, targets: &Targets, now: Timestamp) -> Result<Vec<QueueEntry>> {
        let sql = format!(
            "SELECT e.id, e.period_start, e.first_contact_at, {LATEST_CATEGORY} AS category \
             FROM encounters e WHERE e.class = 'emergency' AND e.status IN ('arrived', 'in_progress') \
               AND e.deleted_at IS NULL AND e.first_contact_at IS NULL AND e.period_start <= $1 \
             ORDER BY e.period_start, e.id"
        );
        let rows = self.attendances(&sql, vec![now.into()]).await?;
        Ok(build_queue(&rows, targets, now))
    }
}

//! A clinician's priority for a waiting-list entry, with a date by which it must be reviewed (NEW-17).
//!
//! Surgical waiting lists carry a clinical priority (P1a within 72 hours, P1b within a month, P2 within a
//! month, P3 within three months, P4 beyond three months) and categories for patients who chose to wait
//! (P5, P6). The meaning of P5 and P6 differs between sources; a clinician should confirm the codes before
//! use. Each priority has a date by which a clinician looks at it again; this lists the ones past it.
//!
//! **Support only (NFR-11).** The priority is recorded and shown. Nothing here, or anywhere else, orders the
//! waiting list, offers a slot or books by it: clinical ordering is a person's decision.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact};
use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The clinical priority codes, in the order of urgency.
pub const CODES: [&str; 7] = ["P1a", "P1b", "P2", "P3", "P4", "P5", "P6"];

#[derive(Debug, Serialize)]
pub struct OverdueReview {
    pub entry_id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub clinical_priority: String,
    pub review_by: NaiveDate,
}

pub struct ClinicalPriorityService {
    db: Pool,
}

impl ClinicalPriorityService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// Set the priority of a waiting or booked entry, and when it is to be reviewed, by a named clinician.
    pub async fn set(
        &self,
        entry_id: Uuid,
        code: &str,
        review_by: NaiveDate,
        clinician: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        self.set_if_unchanged(entry_id, code, review_by, clinician, None, ctx)
            .await
            .map(|_| ())
    }

    /// [`Self::set`] for a caller that read the entry (OFH-1): `expected` is the `updated_at` it saw, and a
    /// different version is refused with `412` before anything changes. Returns the entry's new `updated_at`.
    pub async fn set_if_unchanged(
        &self,
        entry_id: Uuid,
        code: &str,
        review_by: NaiveDate,
        clinician: &str,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<chrono::DateTime<chrono::Utc>> {
        if !CODES.contains(&code) {
            return Err(Error::validation(format!(
                "clinical priority must be one of {}",
                CODES.join(", ")
            )));
        }
        let clinician = named_person(
            clinician,
            "set_by: the clinician setting the priority is required",
        )?;
        let txn = self.db.begin_txn().await?;
        let before = txn
            .one(Stmt::new("SELECT clinical_priority, clinical_priority_review_by, updated_at FROM waitlist_entries \
                 WHERE id = $1 AND status IN ('waiting','booked') FOR UPDATE",
                [entry_id.into()],
            ))
.await?
            .ok_or_else(|| {
                Error::not_found(format!("waiting or booked waiting-list entry {entry_id}"))
            })?;
        let seen: chrono::DateTime<chrono::Utc> = before.col("updated_at")?;
        if !crate::api::rest::concurrency::is_current(seen, expected) {
            return Err(Error::precondition_failed(
                "the waiting-list entry has changed since it was read; read it again",
            ));
        }
        let old_code: Option<String> = before.col("clinical_priority")?;
        let old_review: Option<NaiveDate> = before.col("clinical_priority_review_by")?;
        let updated = txn
            .one(Stmt::new("UPDATE waitlist_entries SET clinical_priority = $2, clinical_priority_set_by = $3, \
               clinical_priority_review_by = $4, updated_at = now() WHERE id = $1 RETURNING updated_at",
            [
                entry_id.into(),
                code.into(),
                clinician.into(),
                review_by.into(),
            ],
        ))
.await?
        .ok_or_else(|| Error::not_found(format!("waitlist entry {entry_id}")))?;
        let new_version: chrono::DateTime<chrono::Utc> = updated.col("updated_at")?;
        AuditLogRepository::log(
            &txn,
            "waitlist_entry",
            entry_id,
            "set_clinical_priority",
            Some(serde_json::json!({ "clinical_priority": old_code, "review_by": old_review })),
            Some(serde_json::json!({ "clinical_priority": code, "review_by": review_by, "set_by": clinician })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(new_version)
    }

    /// Waiting or booked entries whose priority review date is before `today`, the longest overdue first.
    /// At most `fetch` rows (see `crate::paging`).
    pub async fn overdue(&self, today: NaiveDate, fetch: i64) -> Result<Vec<OverdueReview>> {
        let rows = self
            .db
            .all(Stmt::new("SELECT id, person_id, target_service, clinical_priority, clinical_priority_review_by \
                 FROM waitlist_entries \
                 WHERE clinical_priority_review_by < $1 AND status IN ('waiting','booked') \
                 ORDER BY clinical_priority_review_by, id LIMIT $2",
                [today.into(), crate::paging::fetch(fetch).into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(OverdueReview {
                    entry_id: r.col("id")?,
                    person_id: r.col("person_id")?,
                    target_service: r.col("target_service")?,
                    clinical_priority: r.col("clinical_priority")?,
                    review_by: r.col("clinical_priority_review_by")?,
                })
            })
            .collect()
    }
}

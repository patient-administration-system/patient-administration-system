//! Reviewing a patient's communication needs (NEW-20).
//!
//! The Accessible Information Standard (DAPB1605, 2025) adds a sixth step to identify, record, flag, share
//! and meet: review. A patient's recorded needs (large print, easy read, audio, BSL, letter only) are
//! checked with them again from time to time. This records when and by whom, and lists the profiles with
//! needs that were never reviewed or not within the period, oldest first, for a person to follow up.

use crate::db::row::RowExt;
use crate::db::sql::Transact;
use crate::db::sql::{Conn, Stmt};
use chrono::{DateTime, Months, Utc};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The review period when the caller gives none, in months.
pub const DEFAULT_MONTHS: u32 = 12;

#[derive(Debug, Serialize)]
pub struct DueReview {
    pub person_id: Uuid,
    pub accessibility_needs: Vec<String>,
    /// `None` when the needs were never reviewed.
    pub needs_reviewed_at: Option<DateTime<Utc>>,
}

pub struct NeedsReviewService {
    db: DatabaseConnection,
}

impl NeedsReviewService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record that a person reviewed the patient's communication needs with them at `at`.
    pub async fn record_review(
        &self,
        person_id: Uuid,
        reviewed_by: &str,
        at: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            reviewed_by,
            "reviewed_by: the person who reviewed the needs is required",
        )?;
        let txn = self.db.begin_txn().await?;
        let before = txn
            .one(Stmt::new("SELECT needs_reviewed_at FROM patient_communication_profiles WHERE person_id = $1 FOR UPDATE",
                [person_id.into()],
            ))
.await?
            .ok_or_else(|| Error::not_found(format!("communication profile for {person_id}")))?;
        let old: Option<DateTime<Utc>> = before.col("needs_reviewed_at")?;
        txn.run(Stmt::new("UPDATE patient_communication_profiles SET needs_reviewed_at = $2, needs_reviewed_by = $3 WHERE person_id = $1",
            [person_id.into(), at.into(), by.into()],
        ))
.await?;
        AuditLogRepository::log(
            &txn,
            "communication_profile",
            person_id,
            "review_needs",
            Some(serde_json::json!({ "needs_reviewed_at": old })),
            Some(serde_json::json!({ "needs_reviewed_at": at, "needs_reviewed_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// Profiles with recorded needs that were never reviewed, or not in the `months` before `now`; the
    /// never-reviewed first, then the oldest review. At most `fetch` rows (see `crate::paging`).
    pub async fn due(&self, now: DateTime<Utc>, months: u32, fetch: i64) -> Result<Vec<DueReview>> {
        if !(1..=120).contains(&months) {
            return Err(Error::validation("months must be between 1 and 120"));
        }
        let cutoff = now
            .checked_sub_months(Months::new(months))
            .ok_or_else(|| Error::validation("months is too large"))?;
        let rows = self
            .db
            .all(Stmt::new("SELECT person_id, accessibility_needs, needs_reviewed_at FROM patient_communication_profiles \
                 WHERE cardinality(accessibility_needs) > 0 \
                   AND (needs_reviewed_at IS NULL OR needs_reviewed_at < $1) \
                 ORDER BY needs_reviewed_at NULLS FIRST, person_id LIMIT $2",
                [cutoff.into(), crate::paging::fetch(fetch).into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(DueReview {
                    person_id: r.col("person_id")?,
                    accessibility_needs: r.col("accessibility_needs")?,
                    needs_reviewed_at: r.col("needs_reviewed_at")?,
                })
            })
            .collect()
    }
}

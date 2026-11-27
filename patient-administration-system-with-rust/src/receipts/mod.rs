//! Delivery receipts, retries and contact hygiene (NOT-4, CAS-9).
//!
//! - **Receipts.** A carrier reports what happened to a message by the id we gave it (the letter id):
//!   `delivered`, `bounced` (with a short reason code) or `replied`. Each report is stored once per
//!   `(carrier, receipt_id)`, so a retried webhook changes nothing. A `bounced` report on a sent letter marks
//!   the letter `failed`, so the reminder machinery can try another channel and its retry cap counts it.
//! - **Retries back off.** [`retry_delay`] says how long to wait after the nth failed attempt, and a failed
//!   reminder is not tried again before then. After `MAX_FAILED_ATTEMPTS` the letter is given up on.
//! - **Bounces surface.** [`ReceiptService::contact_problems`] lists patients whose messages have bounced since
//!   a person last dealt with them, so the contact details can be fixed. A person resolving one records who
//!   and when; only later bounces count again. This list only *adds* a task: nothing here stops anyone being
//!   offered anything (NFR-11).

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system_domain::arith::in_range;
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const EVENTS: [&str; 3] = ["delivered", "bounced", "replied"];

/// The carrier name a returned paper letter is recorded under (TSC-8).
pub const POST_CARRIER: &str = "post";

/// How long to wait after `failed_attempts` failed attempts before trying again: 5 minutes, then 15, an hour,
/// four hours, twelve; longer than that is "give up" (see `reminders::MAX_FAILED_ATTEMPTS`).
pub fn retry_delay(failed_attempts: i64) -> SignedDuration {
    match failed_attempts {
        i64::MIN..=0 => SignedDuration::ZERO,
        1 => SignedDuration::from_mins(5),
        2 => SignedDuration::from_mins(15),
        3 => SignedDuration::from_hours(1),
        4 => SignedDuration::from_hours(4),
        _ => SignedDuration::from_hours(12),
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Receipt {
    pub letter_id: Uuid,
    pub event: String,
    /// True when this receipt had been stored before and nothing changed.
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ContactProblem {
    pub person_id: Uuid,
    pub bounces: i64,
    pub last_bounce_at: Timestamp,
    pub last_channel: String,
    #[schema(required = true)]
    pub last_reason: Option<String>,
}

pub struct ReceiptService {
    db: Pool,
}

impl ReceiptService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// A paper letter came back undelivered (TSC-8): recorded as the post's `bounced` receipt, so the letter is
    /// marked failed, the patient joins the contact problems and, with nothing else answered, the work queue
    /// asks a person to try by phone. Once per letter; only a printed letter can be returned. `reason` is a
    /// short code such as `gone_away` or `addressee_unknown`.
    pub async fn record_returned(
        &self,
        letter_id: Uuid,
        reason: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Receipt> {
        let channel: String = self
            .db
            .one(Stmt::new(
                "SELECT channel FROM generated_letters WHERE id = $1",
                [letter_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("letter {letter_id}")))?
            .col("channel")?;
        if channel != "print" {
            return Err(Error::conflict(
                "only a printed letter can be returned by post; a carrier reports other bounces",
            ));
        }
        let reason = reason.map(str::trim).filter(|r| !r.is_empty());
        self.record(
            POST_CARRIER,
            &format!("returned:{letter_id}"),
            letter_id,
            "bounced",
            Some(reason.unwrap_or("returned")),
            None,
            ctx,
        )
        .await
    }

    /// Store a carrier's report about a letter. Idempotent by `(carrier, receipt_id)`.
    #[allow(
        clippy::too_many_arguments,
        reason = "one receipt row's columns; a struct would only rename them"
    )]
    pub async fn record(
        &self,
        carrier: &str,
        receipt_id: &str,
        letter_id: Uuid,
        event: &str,
        detail: Option<&str>,
        occurred_at: Option<Timestamp>,
        ctx: &UserContext,
    ) -> Result<Receipt> {
        if !EVENTS.contains(&event) {
            return Err(Error::validation(
                "event must be delivered, bounced or replied",
            ));
        }
        let (carrier, receipt_id) = (carrier.trim(), receipt_id.trim());
        if carrier.is_empty() || receipt_id.is_empty() || receipt_id.len() > 200 {
            return Err(Error::validation(
                "carrier and a receipt_id of at most 200 characters are required",
            ));
        }
        if detail.is_some_and(|d| d.len() > 200 || d.chars().any(char::is_control)) {
            return Err(Error::validation(
                "detail is a short reason code of at most 200 characters",
            ));
        }
        // The receipt, the letter's change and the audit row are one transaction (audit PA-07): if any of them
        // fails nothing is kept, so the carrier's retry of the same receipt finds no row and applies it again.
        // The letter row is locked first, so two copies of one report arriving together are applied once.
        let txn = self.db.begin_txn().await?;
        let known = txn
            .one(Stmt::new(
                "SELECT status FROM generated_letters WHERE id = $1 FOR UPDATE",
                [letter_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("letter {letter_id}")))?;
        let status: String = known.col("status")?;
        let made = txn
            .run(Stmt::new("INSERT INTO message_receipts (id, letter_id, event, carrier, receipt_id, detail, occurred_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (carrier, receipt_id) DO NOTHING",
                [
                    Uuid::new_v4().into(),
                    letter_id.into(),
                    event.into(),
                    carrier.into(),
                    receipt_id.into(),
                    detail.map(String::from).into(),
                    occurred_at.unwrap_or_else(jiff::Timestamp::now).into(),
                ],
            ))
            .await?;
        if made.rows_affected() == 0 {
            txn.rollback().await.map_err(Error::Database)?;
            return Ok(Receipt {
                letter_id,
                event: event.into(),
                duplicate: true,
            });
        }
        if event == "bounced" && status == "sent" {
            txn.run(Stmt::new("UPDATE generated_letters SET status = 'failed', updated_at = now() WHERE id = $1 AND status = 'sent'",
                [letter_id.into()],
            ))
            .await?;
        }
        AuditLogRepository::log_with(
            &txn,
            "generated_letter",
            letter_id,
            &format!("receipt_{event}"),
            None,
            Some(serde_json::json!({ "carrier": carrier })),
            detail,
            None,
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(Receipt {
            letter_id,
            event: event.into(),
            duplicate: false,
        })
    }

    /// Patients whose messages have bounced since a person last dealt with them, most recent first.
    pub async fn contact_problems(
        &self,
        since_days: i64,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<ContactProblem>> {
        let rows = rows.into();
        let since_days = i32::try_from(in_range("since_days", since_days, 1, 3650)?)
            .map_err(|_| Error::validation("since_days must be from 1 to 3650"))?;
        let rows = self
            .db
            .all(Stmt::new("SELECT g.person_id, count(*)::bigint AS bounces, max(r.occurred_at) AS last_at, \
                        (array_agg(g.channel ORDER BY r.occurred_at DESC))[1] AS last_channel, \
                        (array_agg(r.detail ORDER BY r.occurred_at DESC))[1] AS last_reason \
                 FROM message_receipts r JOIN generated_letters g ON g.id = r.letter_id \
                 WHERE r.event = 'bounced' AND r.occurred_at > now() - make_interval(days => $1::int) \
                   AND NOT EXISTS (SELECT 1 FROM contact_problem_resolutions c \
                                   WHERE c.person_id = g.person_id AND c.resolved_at >= r.occurred_at) \
                 GROUP BY g.person_id ORDER BY max(r.occurred_at) DESC, g.person_id LIMIT $2 OFFSET $3",
                [since_days.into(), rows.fetch()?.into(), rows.offset()?.into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(ContactProblem {
                    person_id: r.col("person_id")?,
                    bounces: r.col("bounces")?,
                    last_bounce_at: r.col("last_at")?,
                    last_channel: r.col("last_channel")?,
                    last_reason: r.col("last_reason")?,
                })
            })
            .collect()
    }

    /// A person has dealt with a patient's contact details (fixed a number, confirmed it, rang them).
    pub async fn resolve_problem(
        &self,
        person_id: Uuid,
        resolved_by: &str,
        note: Option<&str>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = crate::validation::named_person(
            resolved_by,
            "a contact problem is resolved by a named person (resolved_by)",
        )?;
        if note.is_some_and(|n| n.len() > 200) {
            return Err(Error::validation("note must be at most 200 characters"));
        }
        // The resolution and its audit row together: a resolution nobody can account for is not kept, and the
        // patient stays on the list (audit PA-07).
        let txn = self.db.begin_txn().await?;
        txn.run(Stmt::new("INSERT INTO contact_problem_resolutions (id, person_id, resolved_by, note, resolved_at) VALUES ($1,$2,$3,$4,$5)",
            [
                Uuid::new_v4().into(),
                person_id.into(),
                by.into(),
                note.map(String::from).into(),
                // The server's clock, like `occurred_at`, so the two are compared on one clock.
                Timestamp::now().into(),
            ],
        ))
        .await?;
        AuditLogRepository::log(
            &txn,
            "patient",
            person_id,
            "contact_problem_resolved",
            None,
            Some(serde_json::json!({ "resolved_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// The receipts for one letter, oldest first.
    pub async fn for_letter(&self, letter_id: Uuid) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .all(Stmt::new("SELECT event, carrier, detail, occurred_at FROM message_receipts WHERE letter_id = $1 ORDER BY occurred_at",
                [letter_id.into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "event": r.col::<String>("event")?,
                    "carrier": r.col::<String>("carrier")?,
                    "detail": r.col::<Option<String>>("detail")?,
                    "occurred_at": r.col::<Timestamp>("occurred_at")?,
                }))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_retry_delay_grows_and_levels_off() {
        let secs: Vec<i64> = (0..=7).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(secs, vec![0, 300, 900, 3600, 14400, 43200, 43200, 43200]);
        assert!(
            secs.windows(2).all(|w| w[0] <= w[1]),
            "never shorter than before"
        );
    }
}

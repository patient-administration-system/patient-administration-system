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

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const EVENTS: [&str; 3] = ["delivered", "bounced", "replied"];

/// How long to wait after `failed_attempts` failed attempts before trying again: 5 minutes, then 15, an hour,
/// four hours, twelve; longer than that is "give up" (see `reminders::MAX_FAILED_ATTEMPTS`).
pub fn retry_delay(failed_attempts: i64) -> Duration {
    match failed_attempts {
        i64::MIN..=0 => Duration::zero(),
        1 => Duration::minutes(5),
        2 => Duration::minutes(15),
        3 => Duration::hours(1),
        4 => Duration::hours(4),
        _ => Duration::hours(12),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub letter_id: Uuid,
    pub event: String,
    /// True when this receipt had been stored before and nothing changed.
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactProblem {
    pub person_id: Uuid,
    pub bounces: i64,
    pub last_bounce_at: DateTime<Utc>,
    pub last_channel: String,
    pub last_reason: Option<String>,
}

pub struct ReceiptService {
    db: DatabaseConnection,
}

impl ReceiptService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Store a carrier's report about a letter. Idempotent by `(carrier, receipt_id)`.
    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        &self,
        carrier: &str,
        receipt_id: &str,
        letter_id: Uuid,
        event: &str,
        detail: Option<&str>,
        occurred_at: Option<DateTime<Utc>>,
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
        let known = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT status FROM generated_letters WHERE id = $1",
                [letter_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("letter {letter_id}")))?;
        let status: String = known.try_get("", "status").map_err(Error::Database)?;
        let made = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO message_receipts (id, letter_id, event, carrier, receipt_id, detail, occurred_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (carrier, receipt_id) DO NOTHING",
                [
                    Uuid::new_v4().into(),
                    letter_id.into(),
                    event.into(),
                    carrier.into(),
                    receipt_id.into(),
                    detail.map(String::from).into(),
                    occurred_at.unwrap_or_else(Utc::now).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        if made.rows_affected() == 0 {
            return Ok(Receipt {
                letter_id,
                event: event.into(),
                duplicate: true,
            });
        }
        if event == "bounced" && status == "sent" {
            self.db
                .execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "UPDATE generated_letters SET status = 'failed', updated_at = now() WHERE id = $1 AND status = 'sent'",
                    [letter_id.into()],
                ))
                .await
                .map_err(Error::Database)?;
        }
        AuditLogRepository::log_with(
            &self.db,
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
        limit: i64,
    ) -> Result<Vec<ContactProblem>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT g.person_id, count(*)::bigint AS bounces, max(r.occurred_at) AS last_at, \
                        (array_agg(g.channel ORDER BY r.occurred_at DESC))[1] AS last_channel, \
                        (array_agg(r.detail ORDER BY r.occurred_at DESC))[1] AS last_reason \
                 FROM message_receipts r JOIN generated_letters g ON g.id = r.letter_id \
                 WHERE r.event = 'bounced' AND r.occurred_at > now() - make_interval(days => $1::int) \
                   AND NOT EXISTS (SELECT 1 FROM contact_problem_resolutions c \
                                   WHERE c.person_id = g.person_id AND c.resolved_at >= r.occurred_at) \
                 GROUP BY g.person_id ORDER BY max(r.occurred_at) DESC LIMIT $2",
                [(since_days.clamp(1, 3650) as i32).into(), limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(ContactProblem {
                    person_id: r.try_get("", "person_id").map_err(Error::Database)?,
                    bounces: r.try_get("", "bounces").map_err(Error::Database)?,
                    last_bounce_at: r.try_get("", "last_at").map_err(Error::Database)?,
                    last_channel: r.try_get("", "last_channel").map_err(Error::Database)?,
                    last_reason: r.try_get("", "last_reason").map_err(Error::Database)?,
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
        let by = resolved_by.trim();
        if by.is_empty() {
            return Err(Error::validation(
                "a contact problem is resolved by a named person (resolved_by)",
            ));
        }
        if note.is_some_and(|n| n.len() > 200) {
            return Err(Error::validation("note must be at most 200 characters"));
        }
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO contact_problem_resolutions (id, person_id, resolved_by, note, resolved_at) VALUES ($1,$2,$3,$4,$5)",
                [
                    Uuid::new_v4().into(),
                    person_id.into(),
                    by.into(),
                    note.map(String::from).into(),
                    // The server's clock, like `occurred_at`, so the two are compared on one clock.
                    Utc::now().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person_id,
            "contact_problem_resolved",
            None,
            Some(serde_json::json!({ "resolved_by": by })),
            ctx,
        )
        .await
    }

    /// The receipts for one letter, oldest first.
    pub async fn for_letter(&self, letter_id: Uuid) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT event, carrier, detail, occurred_at FROM message_receipts WHERE letter_id = $1 ORDER BY occurred_at",
                [letter_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "event": r.try_get::<String>("", "event").map_err(Error::Database)?,
                    "carrier": r.try_get::<String>("", "carrier").map_err(Error::Database)?,
                    "detail": r.try_get::<Option<String>>("", "detail").map_err(Error::Database)?,
                    "occurred_at": r.try_get::<DateTime<Utc>>("", "occurred_at").map_err(Error::Database)?,
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
        let secs: Vec<i64> = (0..=7).map(|n| retry_delay(n).num_seconds()).collect();
        assert_eq!(secs, vec![0, 300, 900, 3600, 14400, 43200, 43200, 43200]);
        assert!(
            secs.windows(2).all(|w| w[0] <= w[1]),
            "never shorter than before"
        );
    }
}

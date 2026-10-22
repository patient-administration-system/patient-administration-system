//! Follow-up target dates and the delayed-follow-up report (NEW-15).
//!
//! A follow-up waiting-list entry can carry the date by which the patient should be seen. Health boards in
//! Wales report every month how many follow-up patients are past that date, booked and not booked, banded by
//! how far past (Digital Health and Care Wales change notice DSCN 2020/18). This module records the target
//! (a named person, audited) and builds that count.
//!
//! - **How far past** is the days past the target as a share of the interval the target set: an entry
//!   added on day 0 with a target of day 28, seen on day 56, is 100% delayed.
//! - **The bands** are on time, under 50%, 50% to 99%, and 100% or more. They follow the change notice as
//!   far as it was read for NEW-15; check them against the current notice before relying on the report.
//! - **No identifiers**, and small cells are withheld as in the analytics feed (CAP-11): a cell under `k` is
//!   hidden, and a single hidden cell hides the smallest other one so it cannot be worked out from the total.

use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::analytics::{Cell, suppress};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The delay bands, in order.
pub const BANDS: [&str; 4] = ["on_time", "under_50", "50_to_99", "100_or_more"];

/// How far past its target an entry is, in whole percent of the target interval (0 when not past it).
pub fn delay_percent(added: NaiveDate, target: NaiveDate, today: NaiveDate) -> i64 {
    let past = (today - target).num_days();
    if past <= 0 {
        return 0;
    }
    // A target on the day the entry was added (or before it) counts as a one-day interval.
    let interval = (target - added).num_days().max(1);
    past * 100 / interval
}

/// The band a delay falls in.
pub fn band(percent: i64) -> &'static str {
    match percent {
        ..=0 => "on_time",
        1..=49 => "under_50",
        50..=99 => "50_to_99",
        _ => "100_or_more",
    }
}

/// The report: one cell per (booked, band), small ones withheld, and the true total.
#[derive(Debug, Serialize)]
pub struct DelayReport {
    pub as_of: NaiveDate,
    pub k: i64,
    /// Every follow-up entry with a target that is waiting or booked.
    pub total: i64,
    pub cells: Vec<Cell>,
}

pub struct FollowUpDelayService {
    db: DatabaseConnection,
}

impl FollowUpDelayService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record the date by which a waiting or booked entry's follow-up is due, with the person who set it.
    pub async fn set_target(
        &self,
        entry_id: Uuid,
        target: NaiveDate,
        set_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let set_by = named_person(set_by, "set_by: the person setting the target is required")?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let before = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT follow_up_target FROM waitlist_entries WHERE id = $1 AND status IN ('waiting','booked') FOR UPDATE",
                [entry_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("waiting or booked waiting-list entry {entry_id}")))?;
        let old: Option<NaiveDate> = before
            .try_get("", "follow_up_target")
            .map_err(Error::Database)?;
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE waitlist_entries SET follow_up_target = $2, follow_up_target_set_by = $3, updated_at = now() \
             WHERE id = $1",
            [entry_id.into(), target.into(), set_by.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "waitlist_entry",
            entry_id,
            "set_follow_up_target",
            Some(serde_json::json!({ "follow_up_target": old })),
            Some(serde_json::json!({ "follow_up_target": target, "set_by": set_by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// The delayed-follow-up count as of `today` (the clinic's date), with cells under `k` withheld.
    pub async fn report(&self, today: NaiveDate, k: i64) -> Result<DelayReport> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT status, created_at, follow_up_target FROM waitlist_entries \
                 WHERE follow_up_target IS NOT NULL AND status IN ('waiting','booked')",
            ))
            .await
            .map_err(Error::Database)?;
        let tz = crate::inbound::messages::timezone();
        let mut counts = std::collections::BTreeMap::<(bool, &str), i64>::new();
        for booked in [false, true] {
            for b in BANDS {
                counts.insert((booked, b), 0);
            }
        }
        for r in &rows {
            let status: String = r.try_get("", "status").map_err(Error::Database)?;
            let added: DateTime<Utc> = r.try_get("", "created_at").map_err(Error::Database)?;
            let target: NaiveDate = r.try_get("", "follow_up_target").map_err(Error::Database)?;
            let pct = delay_percent(added.with_timezone(&tz).date_naive(), target, today);
            *counts.entry((status == "booked", band(pct))).or_insert(0) += 1;
        }
        let total = counts.values().sum();
        let cells = suppress(
            counts
                .into_iter()
                .map(|((booked, b), n)| {
                    (
                        vec![
                            ("booked".to_string(), booked.to_string()),
                            ("band".to_string(), b.to_string()),
                        ],
                        n,
                    )
                })
                .collect(),
            k,
        );
        Ok(DelayReport {
            as_of: today,
            k,
            total,
            cells,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 1, 1).unwrap() + chrono::Duration::days(day)
    }

    #[test]
    fn eight_weeks_waited_against_a_four_week_target_is_100_percent_delayed() {
        assert_eq!(delay_percent(d(0), d(28), d(56)), 100);
        assert_eq!(band(100), "100_or_more");
    }

    #[test]
    fn on_or_before_the_target_is_on_time() {
        assert_eq!(delay_percent(d(0), d(28), d(28)), 0);
        assert_eq!(delay_percent(d(0), d(28), d(1)), 0);
        assert_eq!(band(0), "on_time");
    }

    #[test]
    fn the_bands_meet_without_gaps() {
        assert_eq!(delay_percent(d(0), d(28), d(29)), 3);
        assert_eq!(band(1), "under_50");
        assert_eq!(band(49), "under_50");
        assert_eq!(band(50), "50_to_99");
        assert_eq!(band(99), "50_to_99");
        assert_eq!(delay_percent(d(0), d(28), d(42)), 50);
    }

    #[test]
    fn a_target_on_the_day_it_was_set_counts_as_a_one_day_interval() {
        assert_eq!(delay_percent(d(0), d(0), d(1)), 100);
        assert_eq!(delay_percent(d(5), d(0), d(2)), 200);
    }
}

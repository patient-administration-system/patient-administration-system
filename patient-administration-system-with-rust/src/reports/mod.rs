//! Activity and performance reporting (CORE-7).
//!
//! An operational report for managers: how many appointments there were and how they ended, how full the
//! schedules were, how long patients waited for their appointment, and how long the open pathways have
//! waited. It carries no patient identifier. (For splits by deprivation, age and language, with small cells
//! suppressed, use the analytics feed, CAP-11.)
//!
//! - **Counts reconcile.** Every appointment starting in the range falls in exactly one outcome, so the
//!   outcomes of a row add up to its `total`, and the rows add up to `total_appointments`.
//! - **Utilization is busy slots over all slots** starting in the range, `0` when there are none.
//! - **An empty range gives zeros, never an error.**

use crate::db::row::RowExt;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use std::collections::BTreeMap;

use crate::db::repositories::rtt::RttRepository;
use crate::models::rtt::compute_active_weeks;
use crate::{Error, Result};

const MAX_RANGE_DAYS: i64 = 366;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Day,
    Schedule,
}

impl GroupBy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "day" => Some(Self::Day),
            "schedule" => Some(Self::Schedule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Row {
    pub key: String,
    /// For `schedule` grouping, the service the schedule is for.
    pub service: Option<String>,
    /// Still to happen: booked or proposed.
    pub booked: i64,
    /// Arrived or completed.
    pub attended: i64,
    pub did_not_attend: i64,
    /// Cancelled with three or more days' notice.
    pub cancelled_early: i64,
    /// Cancelled with less.
    pub cancelled_late: i64,
    pub total: i64,
    pub slots: i64,
    pub busy_slots: i64,
    /// `busy_slots / slots`, `0` when there are no slots.
    pub utilization: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub n: i64,
    pub mean: f64,
    pub median: f64,
    pub p90: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub group_by: &'static str,
    pub total_appointments: i64,
    pub total_slots: i64,
    pub busy_slots: i64,
    pub utilization: f64,
    pub rows: Vec<Row>,
    /// Days from booking to the visit, for appointments in the range.
    pub lead_time_days: Stats,
    /// Weeks waited on the pathways open now (not limited to the range).
    pub open_pathway_weeks: Stats,
    pub breaching_pathways: i64,
    /// Weeks waited on each open pathway with the service it waits for. Not serialised: it is for the
    /// tolerances, which judge each service on its own.
    #[serde(skip)]
    pub pathway_weeks: Vec<(String, f64)>,
}

impl Report {
    pub fn to_csv(&self) -> String {
        let mut out = String::from(
            "key,service,booked,attended,did_not_attend,cancelled_early,cancelled_late,total,slots,busy_slots,utilization\n",
        );
        for r in &self.rows {
            let key = csv_field(&r.key);
            let svc = csv_field(r.service.as_deref().unwrap_or(""));
            out.push_str(&format!(
                "{key},{svc},{},{},{},{},{},{},{},{},{:.3}\n",
                r.booked,
                r.attended,
                r.did_not_attend,
                r.cancelled_early,
                r.cancelled_late,
                r.total,
                r.slots,
                r.busy_slots,
                r.utilization
            ));
        }
        out
    }
}

fn csv_field(v: &str) -> String {
    let v = if v.starts_with(['=', '+', '-', '@']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

fn ratio(n: i64, of: i64) -> f64 {
    if of == 0 {
        0.0
    } else {
        ((n as f64 / of as f64) * 1000.0).round() / 1000.0
    }
}

/// Mean, median, 90th percentile (nearest rank) and maximum. All zero for no values.
pub fn stats(mut v: Vec<f64>) -> Stats {
    if v.is_empty() {
        return Stats::default();
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let at = |p: f64| v[(((p * n as f64).ceil() as usize).max(1) - 1).min(n - 1)];
    let round = |x: f64| (x * 100.0).round() / 100.0;
    Stats {
        n: n as i64,
        mean: round(v.iter().sum::<f64>() / n as f64),
        median: round(at(0.5)),
        p90: round(at(0.9)),
        max: round(v[n - 1]),
    }
}

pub struct ReportService {
    db: DatabaseConnection,
}

impl ReportService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn clinic_activity(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        group: GroupBy,
    ) -> Result<Report> {
        let to = to.unwrap_or_else(Utc::now);
        let from = from.unwrap_or_else(|| to - Duration::days(30));
        if to < from {
            return Err(Error::validation("to must not be before from"));
        }
        if to - from > Duration::days(MAX_RANGE_DAYS) {
            return Err(Error::validation(format!(
                "the range may be at most {MAX_RANGE_DAYS} days"
            )));
        }
        let tz = crate::inbound::messages::timezone().name().to_string();
        let (appt_key, slot_key) = match group {
            GroupBy::Day => (
                "to_char(a.start_datetime AT TIME ZONE $3::text, 'YYYY-MM-DD')",
                "to_char(sl.start_datetime AT TIME ZONE $3::text, 'YYYY-MM-DD')",
            ),
            GroupBy::Schedule => (
                "COALESCE(sl.schedule_id::text, 'unscheduled')",
                "sl.schedule_id::text",
            ),
        };
        let params = |extra_tz: bool| {
            let mut v: Vec<sea_orm::Value> = vec![from.into(), to.into()];
            if extra_tz {
                v.push(tz.clone().into());
            }
            v
        };
        let by_day = group == GroupBy::Day;

        let appts = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {appt_key} AS k, \
                       count(*) FILTER (WHERE a.status IN ('booked','proposed'))::bigint AS booked, \
                       count(*) FILTER (WHERE a.status IN ('arrived','fulfilled'))::bigint AS attended, \
                       count(*) FILTER (WHERE a.status = 'no_show')::bigint AS dna, \
                       count(*) FILTER (WHERE a.status = 'cancelled' AND a.notice_days >= 3)::bigint AS early, \
                       count(*) FILTER (WHERE a.status = 'cancelled' AND (a.notice_days IS NULL OR a.notice_days < 3))::bigint AS late, \
                       count(*)::bigint AS total \
                     FROM appointments a LEFT JOIN slots sl ON sl.id = a.slot_id \
                     WHERE a.deleted_at IS NULL AND a.start_datetime >= $1 AND a.start_datetime < $2 GROUP BY 1"
                ),
                params(by_day),
            ))
            .await
            .map_err(Error::Database)?;
        let slots = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {slot_key} AS k, count(*)::bigint AS slots, \
                            count(*) FILTER (WHERE sl.status = 'busy')::bigint AS busy \
                     FROM slots sl WHERE sl.start_datetime >= $1 AND sl.start_datetime < $2 GROUP BY 1"
                ),
                params(by_day),
            ))
            .await
            .map_err(Error::Database)?;

        let mut rows: BTreeMap<String, Row> = BTreeMap::new();
        for r in &appts {
            let key: String = r.col("k")?;
            let row = rows.entry(key.clone()).or_insert_with(|| Row {
                key,
                ..Default::default()
            });
            row.booked = r.col("booked")?;
            row.attended = r.col("attended")?;
            row.did_not_attend = r.col("dna")?;
            row.cancelled_early = r.col("early")?;
            row.cancelled_late = r.col("late")?;
            row.total = r.col("total")?;
        }
        for r in &slots {
            let key: Option<String> = r.col("k")?;
            let Some(key) = key else { continue };
            let row = rows.entry(key.clone()).or_insert_with(|| Row {
                key,
                ..Default::default()
            });
            row.slots = r.col("slots")?;
            row.busy_slots = r.col("busy")?;
        }
        if group == GroupBy::Schedule {
            let ids: Vec<String> = rows
                .keys()
                .filter(|k| *k != "unscheduled")
                .cloned()
                .collect();
            if !ids.is_empty() {
                let svc = self
                    .db
                    .query_all_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "SELECT id::text AS id, service_type FROM schedules WHERE id::text = ANY($1)",
                        [ids.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                for r in &svc {
                    let id: String = r.col("id")?;
                    if let Some(row) = rows.get_mut(&id) {
                        row.service = r.col("service_type")?;
                    }
                }
            }
        }
        let mut rows: Vec<Row> = rows.into_values().collect();
        for r in &mut rows {
            r.utilization = ratio(r.busy_slots, r.slots);
        }
        let total_appointments = rows.iter().map(|r| r.total).sum();
        let total_slots: i64 = rows.iter().map(|r| r.slots).sum();
        let busy_slots: i64 = rows.iter().map(|r| r.busy_slots).sum();

        // Lead time: from booking to the visit, for appointments in the range that were ever booked.
        let leads = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT (extract(epoch FROM (start_datetime - created_at)) / 86400)::float8 AS d FROM appointments \
                 WHERE deleted_at IS NULL AND start_datetime >= $1 AND start_datetime < $2",
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let lead_days: Vec<f64> = leads
            .iter()
            .filter_map(|r| r.try_get::<f64>("", "d").ok())
            .map(|d| d.max(0.0).floor())
            .collect();

        // Open pathways: weeks waited on each, from the clock's own figure.
        let paths = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT id, target_service FROM rtt_pathways WHERE stopped_at IS NULL LIMIT 20000"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let now = Utc::now();
        let mut weeks = Vec::new();
        let mut by_service = Vec::new();
        let mut breaching = 0;
        for p in &paths {
            let id: uuid::Uuid = p.col("id")?;
            let events = RttRepository::list_events_for_pathway(&self.db, id).await?;
            let w = compute_active_weeks(&events, now);
            if let Some(pw) = RttRepository::find_pathway_by_id(&self.db, id).await?
                && pw.is_breaching(&events, now)
            {
                breaching += 1;
            }
            weeks.push(f64::from(w));
            by_service.push((p.col::<String>("target_service")?, f64::from(w)));
        }
        Ok(Report {
            from,
            to,
            group_by: if by_day { "day" } else { "schedule" },
            total_appointments,
            total_slots,
            busy_slots,
            utilization: ratio(busy_slots, total_slots),
            rows,
            lead_time_days: stats(lead_days),
            open_pathway_weeks: stats(weeks),
            breaching_pathways: breaching,
            pathway_weeks: by_service,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_of_nothing_are_zero() {
        let s = stats(vec![]);
        assert_eq!(
            (s.n, s.mean, s.median, s.p90, s.max),
            (0, 0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn test_stats_use_nearest_rank_percentiles() {
        let s = stats((1..=10).map(f64::from).collect());
        assert_eq!(s.n, 10);
        assert_eq!(s.mean, 5.5);
        assert_eq!(s.median, 5.0);
        assert_eq!(s.p90, 9.0);
        assert_eq!(s.max, 10.0);
        assert_eq!(stats(vec![7.0]).median, 7.0);
    }

    #[test]
    fn test_ratio_is_zero_for_nothing_and_rounded_otherwise() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(1, 3), 0.333);
    }

    #[test]
    fn test_csv_defuses_formulas() {
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
    }
}

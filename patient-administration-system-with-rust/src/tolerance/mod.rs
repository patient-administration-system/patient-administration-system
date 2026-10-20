//! Tolerances (red / amber / green): how far a measure may vary from its target.
//!
//! Each measure has a **target**, a **tolerance** (the drift that is still fine) and an **escalate-beyond**
//! limit (the drift past which someone must act). The adverse drift is how far the value has moved in the
//! worse direction from the target:
//!
//! - **green**: adverse drift is within the tolerance.
//! - **amber**: past the tolerance, but within the escalate-beyond limit. Watch it; tell the owner.
//! - **red**: past the limit. Escalate.
//! - **no data**: nothing to measure yet. Never shown as green.
//!
//! Measures cover time (waits, lead time), money (invoices collected) and resources (slot use, bed
//! occupancy, missed appointments). The built-in defaults are starting points and a named person changes
//! them (`PUT /api/tolerances/{metric}`); every change is audited. Nothing here acts on a patient: it is a
//! view for managers, and no score or rank of a person takes part (NFR-11).

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::reports::{GroupBy, ReportService};
use crate::{Error, Result};

/// Which way is worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A higher value is worse (waits, missed appointments).
    HigherIsWorse,
    /// A lower value is worse (slot use, money collected).
    LowerIsWorse,
    /// Drift either way is worse (bed occupancy: too full and too empty both hurt).
    Either,
}

/// The tier a measure is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rag {
    Green,
    Amber,
    Red,
    /// Nothing to measure yet.
    NoData,
}

impl Rag {
    fn rank(self) -> u8 {
        match self {
            Rag::NoData => 0,
            Rag::Green => 1,
            Rag::Amber => 2,
            Rag::Red => 3,
        }
    }
}

/// The three numbers that define a measure's tiers, in the measure's own unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Limits {
    pub target: f64,
    pub tolerance: f64,
    pub escalate_beyond: f64,
}

/// How far `value` has drifted in the worse direction (never negative: better than target is `0`).
pub fn adverse_drift(value: f64, target: f64, direction: Direction) -> f64 {
    match direction {
        Direction::HigherIsWorse => (value - target).max(0.0),
        Direction::LowerIsWorse => (target - value).max(0.0),
        Direction::Either => (value - target).abs(),
    }
}

/// Green within the tolerance, amber past it up to the limit, red beyond.
pub fn classify(value: f64, limits: Limits, direction: Direction) -> Rag {
    let drift = adverse_drift(value, limits.target, direction);
    if drift <= limits.tolerance {
        Rag::Green
    } else if drift <= limits.escalate_beyond {
        Rag::Amber
    } else {
        Rag::Red
    }
}

/// One measure the system can read, with its built-in defaults.
#[derive(Debug, Clone, Copy)]
pub struct Metric {
    pub key: &'static str,
    /// `time`, `money` or `resources`.
    pub kind: &'static str,
    pub label: &'static str,
    /// `weeks`, `days` or `share` (a fraction from 0 to 1).
    pub unit: &'static str,
    pub direction: Direction,
    pub default: Limits,
}

const fn limits(target: f64, tolerance: f64, escalate_beyond: f64) -> Limits {
    Limits {
        target,
        tolerance,
        escalate_beyond,
    }
}

/// Every measure, in the order they are shown.
pub const METRICS: [Metric; 6] = [
    Metric {
        key: "wait_p90_weeks",
        kind: "time",
        label: "Wait on open pathways, 90th percentile",
        unit: "weeks",
        direction: Direction::HigherIsWorse,
        default: limits(18.0, 2.0, 8.0),
    },
    Metric {
        key: "lead_time_median_days",
        kind: "time",
        label: "Median days from booking to visit",
        unit: "days",
        direction: Direction::HigherIsWorse,
        default: limits(14.0, 7.0, 21.0),
    },
    Metric {
        key: "did_not_attend_rate",
        kind: "resources",
        label: "Appointments missed without notice",
        unit: "share",
        direction: Direction::HigherIsWorse,
        default: limits(0.08, 0.02, 0.06),
    },
    Metric {
        key: "slot_utilisation",
        kind: "resources",
        label: "Slots in use",
        unit: "share",
        direction: Direction::LowerIsWorse,
        default: limits(0.85, 0.05, 0.15),
    },
    Metric {
        key: "bed_occupancy",
        kind: "resources",
        label: "Beds occupied",
        unit: "share",
        direction: Direction::Either,
        default: limits(0.85, 0.05, 0.10),
    },
    Metric {
        key: "invoice_collection_rate",
        kind: "money",
        label: "Invoiced money collected",
        unit: "share",
        direction: Direction::LowerIsWorse,
        default: limits(0.95, 0.03, 0.10),
    },
];

pub fn metric(key: &str) -> Option<&'static Metric> {
    METRICS.iter().find(|m| m.key == key)
}

/// A measure's settings as they stand now.
#[derive(Debug, Clone, Serialize)]
pub struct Setting {
    pub metric: &'static str,
    pub kind: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub direction: Direction,
    #[serde(flatten)]
    pub limits: Limits,
    /// False while the built-in default applies.
    pub customised: bool,
    pub set_by: Option<String>,
    pub note: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A measure's value against its tiers.
#[derive(Debug, Clone, Serialize)]
pub struct Reading {
    pub metric: &'static str,
    pub kind: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub direction: Direction,
    #[serde(flatten)]
    pub limits: Limits,
    pub value: Option<f64>,
    /// How far the value is from the target in the worse direction.
    pub adverse_drift: Option<f64>,
    pub rag: Rag,
    /// How many things the value is based on (appointments, pathways, slots, beds, invoices).
    pub sample: i64,
    pub customised: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// The worst tier among measures that have data; `no_data` when none do.
    pub overall: Rag,
    pub readings: Vec<Reading>,
}

pub struct ToleranceService {
    db: DatabaseConnection,
    reports: ReportService,
}

fn check(limits: Limits) -> Result<()> {
    let all = [limits.target, limits.tolerance, limits.escalate_beyond];
    if all.iter().any(|v| !v.is_finite()) {
        return Err(Error::validation(
            "target, tolerance and limit must be numbers",
        ));
    }
    if limits.tolerance < 0.0 {
        return Err(Error::validation("the tolerance cannot be negative"));
    }
    if limits.escalate_beyond < limits.tolerance {
        return Err(Error::validation(
            "the escalate-beyond limit must be at least the tolerance (amber sits between them)",
        ));
    }
    Ok(())
}

impl ToleranceService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            reports: ReportService::new(db.clone()),
            db,
        }
    }

    /// Every measure with the settings that apply: the default, or what a named person set.
    pub async fn settings(&self) -> Result<Vec<Setting>> {
        let rows = self
            .db
            .query_all(Statement::from_string(
                DbBackend::Postgres,
                "SELECT metric, target, tolerance, escalate_beyond, set_by, note, updated_at FROM tolerances"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let g = Error::Database;
        let mut out = Vec::new();
        for m in METRICS {
            let found = rows
                .iter()
                .find(|r| r.try_get::<String>("", "metric").is_ok_and(|k| k == m.key));
            let setting = match found {
                Some(r) => Setting {
                    metric: m.key,
                    kind: m.kind,
                    label: m.label,
                    unit: m.unit,
                    direction: m.direction,
                    limits: Limits {
                        target: r.try_get("", "target").map_err(g)?,
                        tolerance: r.try_get("", "tolerance").map_err(g)?,
                        escalate_beyond: r.try_get("", "escalate_beyond").map_err(g)?,
                    },
                    customised: true,
                    set_by: r.try_get("", "set_by").ok(),
                    note: r.try_get("", "note").ok(),
                    updated_at: r.try_get("", "updated_at").ok(),
                },
                None => Setting {
                    metric: m.key,
                    kind: m.kind,
                    label: m.label,
                    unit: m.unit,
                    direction: m.direction,
                    limits: m.default,
                    customised: false,
                    set_by: None,
                    note: None,
                    updated_at: None,
                },
            };
            out.push(setting);
        }
        Ok(out)
    }

    /// A named person sets a measure's target, tolerance and limit. Audited.
    pub async fn set(
        &self,
        key: &str,
        limits: Limits,
        set_by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        let by = set_by.trim();
        if by.is_empty() {
            return Err(Error::validation(
                "name the person setting the tolerance (set_by)",
            ));
        }
        check(limits)?;
        let before = self
            .settings()
            .await?
            .into_iter()
            .find(|s| s.metric == m.key)
            .map(|s| s.limits);
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO tolerances (metric, target, tolerance, escalate_beyond, set_by, note) \
                 VALUES ($1,$2,$3,$4,$5,$6) \
                 ON CONFLICT (metric) DO UPDATE SET target = $2, tolerance = $3, escalate_beyond = $4, \
                   set_by = $5, note = $6, updated_at = now()",
                [
                    m.key.into(),
                    limits.target.into(),
                    limits.tolerance.into(),
                    limits.escalate_beyond.into(),
                    by.into(),
                    note.trim().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "tolerance",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, m.key.as_bytes()),
            "set",
            before.and_then(|l| serde_json::to_value(l).ok()),
            serde_json::to_value(limits).ok(),
            ctx,
        )
        .await
    }

    /// Put a measure back to its built-in default. Audited.
    pub async fn reset(&self, key: &str, ctx: &UserContext) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "DELETE FROM tolerances WHERE metric = $1",
                [m.key.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "tolerance",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, m.key.as_bytes()),
            "reset",
            None,
            serde_json::to_value(m.default).ok(),
            ctx,
        )
        .await
    }

    /// Read every measure over the last `days` days (waits are as of now) and put it in its tier.
    pub async fn status(&self, days: i64) -> Result<Status> {
        let days = days.clamp(1, 365);
        let to = Utc::now();
        let from = to - Duration::days(days);
        let report = self
            .reports
            .clinic_activity(Some(from), Some(to), GroupBy::Day)
            .await?;
        let attended: i64 = report.rows.iter().map(|r| r.attended).sum();
        let missed: i64 = report.rows.iter().map(|r| r.did_not_attend).sum();
        let (beds, occupied) = self.bed_counts().await?;
        let (invoiced, collected, invoices) = self.money(from, to).await?;

        // (value, sample) per measure; None when there is nothing to measure.
        let measured = |key: &str| -> (Option<f64>, i64) {
            match key {
                "wait_p90_weeks" => (
                    (report.open_pathway_weeks.n > 0).then_some(report.open_pathway_weeks.p90),
                    report.open_pathway_weeks.n,
                ),
                "lead_time_median_days" => (
                    (report.lead_time_days.n > 0).then_some(report.lead_time_days.median),
                    report.lead_time_days.n,
                ),
                "did_not_attend_rate" => (
                    (attended + missed > 0).then(|| missed as f64 / (attended + missed) as f64),
                    attended + missed,
                ),
                "slot_utilisation" => (
                    (report.total_slots > 0).then_some(report.utilization),
                    report.total_slots,
                ),
                "bed_occupancy" => ((beds > 0).then(|| occupied as f64 / beds as f64), beds),
                "invoice_collection_rate" => (
                    (invoiced > 0.0).then(|| (collected / invoiced).min(1.0)),
                    invoices,
                ),
                _ => (None, 0),
            }
        };

        let settings = self.settings().await?;
        let mut readings = Vec::new();
        for s in settings {
            let (value, sample) = measured(s.metric);
            let rag = value.map_or(Rag::NoData, |v| classify(v, s.limits, s.direction));
            readings.push(Reading {
                metric: s.metric,
                kind: s.kind,
                label: s.label,
                unit: s.unit,
                direction: s.direction,
                limits: s.limits,
                value: value.map(|v| (v * 1000.0).round() / 1000.0),
                adverse_drift: value.map(|v| {
                    (adverse_drift(v, s.limits.target, s.direction) * 1000.0).round() / 1000.0
                }),
                rag,
                sample,
                customised: s.customised,
            });
        }
        let overall = readings
            .iter()
            .map(|r| r.rag)
            .max_by_key(|r| r.rank())
            .unwrap_or(Rag::NoData);
        Ok(Status {
            from,
            to,
            overall,
            readings,
        })
    }

    async fn bed_counts(&self) -> Result<(i64, i64)> {
        let row = self
            .db
            .query_one(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS beds, count(*) FILTER (WHERE status = 'occupied')::bigint AS occupied FROM beds"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let Some(r) = row else { return Ok((0, 0)) };
        Ok((
            r.try_get("", "beds").map_err(Error::Database)?,
            r.try_get("", "occupied").map_err(Error::Database)?,
        ))
    }

    /// Money invoiced and collected on invoices finalised in the window. Amounts are summed as written, so
    /// this assumes one currency; a second currency in the window would be added into the same total.
    async fn money(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<(f64, f64, i64)> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT COALESCE(sum(i.total_value::numeric), 0)::float8 AS invoiced, \
                        COALESCE((SELECT sum(p.amount_value::numeric) FROM payments p \
                                  JOIN invoices i2 ON i2.id = p.invoice_id \
                                  WHERE i2.status <> 'void' AND i2.finalized_at >= $1 AND i2.finalized_at < $2), 0)::float8 AS collected, \
                        count(*)::bigint AS n \
                 FROM invoices i WHERE i.status <> 'void' AND i.finalized_at >= $1 AND i.finalized_at < $2",
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let Some(r) = row else {
            return Ok((0.0, 0.0, 0));
        };
        Ok((
            r.try_get("", "invoiced").map_err(Error::Database)?,
            r.try_get("", "collected").map_err(Error::Database)?,
            r.try_get("", "n").map_err(Error::Database)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = limits(10.0, 2.0, 5.0);

    #[test]
    fn higher_is_worse_tiers() {
        let d = Direction::HigherIsWorse;
        assert_eq!(classify(8.0, L, d), Rag::Green, "better than target");
        assert_eq!(classify(12.0, L, d), Rag::Green, "exactly at the tolerance");
        assert_eq!(classify(12.1, L, d), Rag::Amber);
        assert_eq!(classify(15.0, L, d), Rag::Amber, "exactly at the limit");
        assert_eq!(classify(15.1, L, d), Rag::Red);
    }

    #[test]
    fn lower_is_worse_tiers() {
        let d = Direction::LowerIsWorse;
        assert_eq!(
            classify(14.0, L, d),
            Rag::Green,
            "above target is not a problem"
        );
        assert_eq!(classify(8.0, L, d), Rag::Green);
        assert_eq!(classify(7.9, L, d), Rag::Amber);
        assert_eq!(classify(4.9, L, d), Rag::Red);
    }

    #[test]
    fn either_direction_counts_both_ways() {
        let d = Direction::Either;
        assert_eq!(classify(11.0, L, d), Rag::Green);
        assert_eq!(classify(9.0, L, d), Rag::Green);
        assert_eq!(classify(13.0, L, d), Rag::Amber);
        assert_eq!(classify(7.0, L, d), Rag::Amber);
        assert_eq!(classify(16.0, L, d), Rag::Red);
        assert_eq!(classify(4.0, L, d), Rag::Red);
    }

    #[test]
    fn a_zero_tolerance_leaves_no_slack() {
        let l = limits(10.0, 0.0, 1.0);
        assert_eq!(classify(10.0, l, Direction::HigherIsWorse), Rag::Green);
        assert_eq!(classify(10.5, l, Direction::HigherIsWorse), Rag::Amber);
    }

    #[test]
    fn limits_must_be_ordered_and_finite() {
        assert!(check(limits(1.0, 0.1, 0.5)).is_ok());
        assert!(
            check(limits(1.0, 0.5, 0.5)).is_ok(),
            "no amber band is allowed"
        );
        assert!(check(limits(1.0, -0.1, 0.5)).is_err());
        assert!(check(limits(1.0, 0.5, 0.4)).is_err());
        assert!(check(limits(f64::NAN, 0.5, 0.6)).is_err());
        assert!(check(limits(1.0, 0.5, f64::INFINITY)).is_err());
    }

    #[test]
    fn every_default_is_valid_and_keys_are_unique() {
        let mut keys = std::collections::HashSet::new();
        for m in METRICS {
            assert!(check(m.default).is_ok(), "{}", m.key);
            assert!(keys.insert(m.key), "duplicate {}", m.key);
            assert!(metric(m.key).is_some());
        }
    }

    #[test]
    fn the_worst_tier_wins_and_no_data_never_beats_green() {
        let worst = |tiers: &[Rag]| tiers.iter().copied().max_by_key(|r| r.rank());
        assert_eq!(worst(&[Rag::Green, Rag::Amber, Rag::Red]), Some(Rag::Red));
        assert_eq!(worst(&[Rag::Green, Rag::NoData]), Some(Rag::Green));
        assert_eq!(worst(&[Rag::NoData]), Some(Rag::NoData));
    }
}

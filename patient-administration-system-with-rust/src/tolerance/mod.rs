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
//! them (`PUT /api/tolerances/{metric}`); every change is audited. A tolerance can also be set for one service
//! or one ward, and where none is set the whole service's applies. When a measure goes red an alert opens (one
//! per measure and scope, and a `ToleranceAlertRaised` event); it closes when the measure is no longer red. Nothing here acts on a patient: it is a
//! view for managers, and no score or rank of a person takes part (NFR-11).

use crate::db::row::RowExt;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
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

/// What a tolerance applies to. `All` is the whole service; the others narrow it to one service or ward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    All,
    /// One service, by its service type (the schedule's `service_type`, the pathway's target service).
    Service,
    /// One ward, by its code.
    Ward,
}

impl ScopeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeKind::All => "all",
            ScopeKind::Service => "service",
            ScopeKind::Ward => "ward",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "all" => Some(ScopeKind::All),
            "service" => Some(ScopeKind::Service),
            "ward" => Some(ScopeKind::Ward),
            _ => None,
        }
    }
}

/// What a tolerance or a reading is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub kind: ScopeKind,
    /// Empty for `All`.
    pub key: String,
}

impl Scope {
    pub fn all() -> Self {
        Scope {
            kind: ScopeKind::All,
            key: String::new(),
        }
    }

    /// A scope from the optional `service` and `ward` of a request. Naming both is an error.
    pub fn from_parts(service: Option<&str>, ward: Option<&str>) -> Result<Self> {
        fn clean(v: Option<&str>) -> Option<&str> {
            v.map(str::trim).filter(|v| !v.is_empty())
        }
        match (clean(service), clean(ward)) {
            (None, None) => Ok(Scope::all()),
            (Some(s), None) => Ok(Scope {
                kind: ScopeKind::Service,
                key: s.to_string(),
            }),
            (None, Some(w)) => Ok(Scope {
                kind: ScopeKind::Ward,
                key: w.to_string(),
            }),
            (Some(_), Some(_)) => Err(Error::validation(
                "a tolerance is for one service or one ward, not both",
            )),
        }
    }

    fn uuid(&self, metric: &str) -> Uuid {
        let name = if self.kind == ScopeKind::All {
            metric.to_string()
        } else {
            format!("{metric}|{}|{}", self.kind.as_str(), self.key)
        };
        Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
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
    /// What this measure can also be judged by, besides the whole service: `Service`, `Ward`, or `All` when
    /// it can only be read for the whole service.
    pub narrows_to: ScopeKind,
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
        narrows_to: ScopeKind::Service,
    },
    Metric {
        key: "lead_time_median_days",
        kind: "time",
        label: "Median days from booking to visit",
        unit: "days",
        direction: Direction::HigherIsWorse,
        default: limits(14.0, 7.0, 21.0),
        narrows_to: ScopeKind::Service,
    },
    Metric {
        key: "did_not_attend_rate",
        kind: "resources",
        label: "Appointments missed without notice",
        unit: "share",
        direction: Direction::HigherIsWorse,
        default: limits(0.08, 0.02, 0.06),
        narrows_to: ScopeKind::Service,
    },
    Metric {
        key: "slot_utilisation",
        kind: "resources",
        label: "Slots in use",
        unit: "share",
        direction: Direction::LowerIsWorse,
        default: limits(0.85, 0.05, 0.15),
        narrows_to: ScopeKind::Service,
    },
    Metric {
        key: "bed_occupancy",
        kind: "resources",
        label: "Beds occupied",
        unit: "share",
        direction: Direction::Either,
        default: limits(0.85, 0.05, 0.10),
        narrows_to: ScopeKind::Ward,
    },
    Metric {
        key: "invoice_collection_rate",
        kind: "money",
        label: "Invoiced money collected",
        unit: "share",
        direction: Direction::LowerIsWorse,
        default: limits(0.95, 0.03, 0.10),
        narrows_to: ScopeKind::All,
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
    pub scope_kind: ScopeKind,
    pub scope_key: String,
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
    pub scope_kind: ScopeKind,
    pub scope_key: String,
    #[serde(flatten)]
    pub limits: Limits,
    pub value: Option<f64>,
    /// How far the value is from the target in the worse direction.
    pub adverse_drift: Option<f64>,
    pub rag: Rag,
    /// How many things the value is based on (appointments, pathways, slots, beds, invoices).
    pub sample: i64,
    /// True when this scope, or this measure, has limits a person set; false while a default applies.
    pub customised: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// The worst tier among every reading that has data; `no_data` when none do.
    pub overall: Rag,
    /// Each measure for the whole service.
    pub readings: Vec<Reading>,
    /// The measures that can be read for one service, for each service that has data (or its own tolerance).
    pub by_service: Vec<Reading>,
    /// Bed occupancy for each ward.
    pub by_ward: Vec<Reading>,
}

/// An alert: a measure that went past its escalate-beyond limit. One stays open while the measure stays red.
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub id: Uuid,
    pub metric: String,
    pub label: Option<&'static str>,
    pub unit: Option<&'static str>,
    pub scope_kind: String,
    pub scope_key: String,
    pub value: f64,
    pub target: f64,
    pub tolerance: f64,
    pub escalate_beyond: f64,
    pub raised_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    /// Set when the measure is no longer red.
    pub cleared_at: Option<DateTime<Utc>>,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub note: String,
}

/// What one check did.
#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    /// Readings looked at.
    pub checked: usize,
    /// New alerts.
    pub raised: usize,
    /// Alerts closed because the measure is no longer red.
    pub cleared: usize,
    /// Alerts open now.
    pub open: i64,
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

/// A scope fits a measure when it is the whole service, or the narrowing the measure allows.
fn check_scope(m: &Metric, scope: &Scope) -> Result<()> {
    if scope.kind == ScopeKind::All {
        return Ok(());
    }
    if m.narrows_to != scope.kind {
        return Err(Error::validation(match m.narrows_to {
            ScopeKind::All => format!("{} is read for the whole service only", m.key),
            ScopeKind::Service => format!("{} is read per service, not per ward", m.key),
            ScopeKind::Ward => format!("{} is read per ward, not per service", m.key),
        }));
    }
    if scope.key.len() > 120 {
        return Err(Error::validation("the service or ward name is too long"));
    }
    Ok(())
}

/// A stored tolerance row.
struct Row {
    scope: Scope,
    metric: String,
    limits: Limits,
    set_by: Option<String>,
    note: Option<String>,
    updated_at: Option<DateTime<Utc>>,
}

/// One value to put in a tier.
struct Measure {
    m: &'static Metric,
    scope: Scope,
    value: Option<f64>,
    sample: i64,
}

fn rounded(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn share(part: i64, of: i64) -> Option<f64> {
    (of > 0).then(|| part as f64 / of as f64)
}

impl ToleranceService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            reports: ReportService::new(db.clone()),
            db,
        }
    }

    async fn rows(&self) -> Result<Vec<Row>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT metric, scope_kind, scope_key, target, tolerance, escalate_beyond, set_by, note, updated_at \
                 FROM tolerances ORDER BY metric, scope_kind, scope_key"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in &rows {
            let kind: String = r.col("scope_kind")?;
            let Some(kind) = ScopeKind::parse(&kind) else {
                continue;
            };
            out.push(Row {
                scope: Scope {
                    kind,
                    key: r.col("scope_key")?,
                },
                metric: r.col("metric")?,
                limits: Limits {
                    target: r.col("target")?,
                    tolerance: r.col("tolerance")?,
                    escalate_beyond: r.col("escalate_beyond")?,
                },
                set_by: r.try_get("", "set_by").ok(),
                note: r.try_get("", "note").ok(),
                updated_at: r.try_get("", "updated_at").ok(),
            });
        }
        Ok(out)
    }

    /// The limits that apply to a measure in a scope: the scope's own, else the whole service's, else the
    /// built-in default. The flag says whether a person set the one used.
    fn resolve(rows: &[Row], m: &Metric, scope: &Scope) -> (Limits, bool) {
        let find = |s: &Scope| rows.iter().find(|r| r.metric == m.key && &r.scope == s);
        find(scope)
            .or_else(|| find(&Scope::all()))
            .map_or((m.default, false), |r| (r.limits, true))
    }

    /// Every measure with the settings that apply to the whole service, then each setting a person made for
    /// one service or ward.
    pub async fn settings(&self) -> Result<Vec<Setting>> {
        let rows = self.rows().await?;
        let make = |m: &Metric, row: Option<&Row>, scope: Scope| Setting {
            metric: m.key,
            kind: m.kind,
            label: m.label,
            unit: m.unit,
            direction: m.direction,
            scope_kind: scope.kind,
            scope_key: scope.key,
            limits: row.map_or(m.default, |r| r.limits),
            customised: row.is_some(),
            set_by: row.and_then(|r| r.set_by.clone()),
            note: row.and_then(|r| r.note.clone()),
            updated_at: row.and_then(|r| r.updated_at),
        };
        let mut out = Vec::new();
        for m in METRICS {
            let whole = rows
                .iter()
                .find(|r| r.metric == m.key && r.scope.kind == ScopeKind::All);
            out.push(make(&m, whole, Scope::all()));
        }
        for r in rows.iter().filter(|r| r.scope.kind != ScopeKind::All) {
            if let Some(m) = metric(&r.metric) {
                out.push(make(m, Some(r), r.scope.clone()));
            }
        }
        Ok(out)
    }

    /// A named person sets a measure's target, tolerance and limit, for the whole service or for one service
    /// or ward. Audited.
    pub async fn set(
        &self,
        key: &str,
        scope: &Scope,
        limits: Limits,
        set_by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        check_scope(m, scope)?;
        let by = crate::validation::named_person(
            set_by,
            "name the person setting the tolerance (set_by)",
        )?;
        check(limits)?;
        let before = self
            .rows()
            .await?
            .into_iter()
            .find(|r| r.metric == m.key && &r.scope == scope)
            .map(|r| r.limits);
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO tolerances (metric, scope_kind, scope_key, target, tolerance, escalate_beyond, set_by, note) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
                 ON CONFLICT (metric, scope_kind, scope_key) DO UPDATE SET target = $4, tolerance = $5, \
                   escalate_beyond = $6, set_by = $7, note = $8, updated_at = now()",
                [
                    m.key.into(),
                    scope.kind.as_str().into(),
                    scope.key.clone().into(),
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
            scope.uuid(m.key),
            "set",
            before.and_then(|l| serde_json::to_value(l).ok()),
            serde_json::to_value(limits).ok(),
            ctx,
        )
        .await
    }

    /// Take away what a person set: the scope goes back to the whole service's limits, and the whole service
    /// back to the built-in default. Audited.
    pub async fn reset(&self, key: &str, scope: &Scope, ctx: &UserContext) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        check_scope(m, scope)?;
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "DELETE FROM tolerances WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3",
                [
                    m.key.into(),
                    scope.kind.as_str().into(),
                    scope.key.clone().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "tolerance",
            scope.uuid(m.key),
            "reset",
            None,
            serde_json::to_value(m.default).ok(),
            ctx,
        )
        .await
    }

    /// Read every measure over the last `days` days (waits are as of now), for the whole service and then for
    /// each service and ward, and put each in its tier.
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

        let mut measures = Vec::new();
        let mut whole = |key: &str, value: Option<f64>, sample: i64| {
            if let Some(m) = metric(key) {
                measures.push(Measure {
                    m,
                    scope: Scope::all(),
                    value,
                    sample,
                });
            }
        };
        whole(
            "wait_p90_weeks",
            (report.open_pathway_weeks.n > 0).then_some(report.open_pathway_weeks.p90),
            report.open_pathway_weeks.n,
        );
        whole(
            "lead_time_median_days",
            (report.lead_time_days.n > 0).then_some(report.lead_time_days.median),
            report.lead_time_days.n,
        );
        whole(
            "did_not_attend_rate",
            share(missed, attended + missed),
            attended + missed,
        );
        whole(
            "slot_utilisation",
            (report.total_slots > 0).then_some(report.utilization),
            report.total_slots,
        );
        whole("bed_occupancy", share(occupied, beds), beds);
        whole(
            "invoice_collection_rate",
            (invoiced > 0.0).then(|| (collected / invoiced).min(1.0)),
            invoices,
        );
        measures.extend(self.service_measures(from, to, &report).await?);
        measures.extend(self.ward_measures().await?);

        let rows = self.rows().await?;
        // A scope someone gave its own tolerance is always shown, even with nothing to measure yet.
        for r in rows.iter().filter(|r| r.scope.kind != ScopeKind::All) {
            let Some(m) = metric(&r.metric) else { continue };
            if !measures
                .iter()
                .any(|x| x.m.key == m.key && x.scope == r.scope)
            {
                measures.push(Measure {
                    m,
                    scope: r.scope.clone(),
                    value: None,
                    sample: 0,
                });
            }
        }
        let mut readings = Vec::new();
        let mut by_service = Vec::new();
        let mut by_ward = Vec::new();
        for x in measures {
            let (limits, customised) = Self::resolve(&rows, x.m, &x.scope);
            let reading = Reading {
                metric: x.m.key,
                kind: x.m.kind,
                label: x.m.label,
                unit: x.m.unit,
                direction: x.m.direction,
                scope_kind: x.scope.kind,
                scope_key: x.scope.key.clone(),
                limits,
                value: x.value.map(rounded),
                adverse_drift: x
                    .value
                    .map(|v| rounded(adverse_drift(v, limits.target, x.m.direction))),
                rag: x
                    .value
                    .map_or(Rag::NoData, |v| classify(v, limits, x.m.direction)),
                sample: x.sample,
                customised,
            };
            match x.scope.kind {
                ScopeKind::All => readings.push(reading),
                ScopeKind::Service => by_service.push(reading),
                ScopeKind::Ward => by_ward.push(reading),
            }
        }
        by_service.sort_by(|a, b| (&a.scope_key, a.metric).cmp(&(&b.scope_key, b.metric)));
        by_ward.sort_by(|a, b| (&a.scope_key, a.metric).cmp(&(&b.scope_key, b.metric)));
        let overall = readings
            .iter()
            .chain(&by_service)
            .chain(&by_ward)
            .map(|r| r.rag)
            .max_by_key(|r| r.rank())
            .unwrap_or(Rag::NoData);
        Ok(Status {
            from,
            to,
            overall,
            readings,
            by_service,
            by_ward,
        })
    }

    /// The four service-level measures for each service that has appointments, slots or open pathways.
    async fn service_measures(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        report: &crate::reports::Report,
    ) -> Result<Vec<Measure>> {
        let q = |sql: &str| {
            Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql.to_string(),
                [from.into(), to.into()],
            )
        };
        let visits = self
            .db
            .query_all_raw(q(
                "SELECT sch.service_type AS svc, \
                   count(*) FILTER (WHERE a.status IN ('arrived','fulfilled'))::bigint AS attended, \
                   count(*) FILTER (WHERE a.status = 'no_show')::bigint AS missed, \
                   array_agg(floor(greatest(0, extract(epoch FROM (a.start_datetime - a.created_at)) / 86400))::float8) AS leads \
                 FROM appointments a JOIN slots sl ON sl.id = a.slot_id JOIN schedules sch ON sch.id = sl.schedule_id \
                 WHERE a.deleted_at IS NULL AND sch.service_type IS NOT NULL \
                   AND a.start_datetime >= $1 AND a.start_datetime < $2 GROUP BY 1",
            ))
            .await
            .map_err(Error::Database)?;
        let slots = self
            .db
            .query_all_raw(q(
                "SELECT sch.service_type AS svc, count(*)::bigint AS slots, \
                   count(*) FILTER (WHERE sl.status = 'busy')::bigint AS busy \
                 FROM slots sl JOIN schedules sch ON sch.id = sl.schedule_id \
                 WHERE sch.service_type IS NOT NULL AND sl.start_datetime >= $1 AND sl.start_datetime < $2 GROUP BY 1",
            ))
            .await
            .map_err(Error::Database)?;

        use std::collections::BTreeMap;
        #[derive(Default)]
        struct Acc {
            attended: i64,
            missed: i64,
            leads: Vec<f64>,
            slots: i64,
            busy: i64,
            waits: Vec<f64>,
        }
        let mut by: BTreeMap<String, Acc> = BTreeMap::new();
        for r in &visits {
            let acc = by.entry(r.col("svc")?).or_default();
            acc.attended = r.col("attended")?;
            acc.missed = r.col("missed")?;
            acc.leads = r.col("leads")?;
        }
        for r in &slots {
            let acc = by.entry(r.col("svc")?).or_default();
            acc.slots = r.col("slots")?;
            acc.busy = r.col("busy")?;
        }
        for (svc, weeks) in &report.pathway_weeks {
            by.entry(svc.clone()).or_default().waits.push(*weeks);
        }

        let mut out = Vec::new();
        for (svc, acc) in by {
            let scope = Scope {
                kind: ScopeKind::Service,
                key: svc,
            };
            let mut add = |key: &str, value: Option<f64>, sample: i64| {
                if let (Some(m), true) = (metric(key), sample > 0) {
                    out.push(Measure {
                        m,
                        scope: scope.clone(),
                        value,
                        sample,
                    });
                }
            };
            let waits = crate::reports::stats(acc.waits);
            add("wait_p90_weeks", Some(waits.p90), waits.n);
            let leads = crate::reports::stats(acc.leads);
            add("lead_time_median_days", Some(leads.median), leads.n);
            add(
                "did_not_attend_rate",
                share(acc.missed, acc.attended + acc.missed),
                acc.attended + acc.missed,
            );
            add("slot_utilisation", share(acc.busy, acc.slots), acc.slots);
        }
        Ok(out)
    }

    async fn ward_measures(&self) -> Result<Vec<Measure>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT w.code AS ward, count(*)::bigint AS beds, \
                   count(*) FILTER (WHERE b.status = 'occupied')::bigint AS occupied \
                 FROM beds b JOIN rooms r ON r.id = b.room_id JOIN wards w ON w.id = r.ward_id GROUP BY w.code"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let Some(m) = metric("bed_occupancy") else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for r in &rows {
            let beds: i64 = r.col("beds")?;
            let occupied: i64 = r.col("occupied")?;
            out.push(Measure {
                m,
                scope: Scope {
                    kind: ScopeKind::Ward,
                    key: r.col("ward")?,
                },
                value: share(occupied, beds),
                sample: beds,
            });
        }
        Ok(out)
    }

    async fn bed_counts(&self) -> Result<(i64, i64)> {
        let row = self
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS beds, count(*) FILTER (WHERE status = 'occupied')::bigint AS occupied FROM beds"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let Some(r) = row else { return Ok((0, 0)) };
        Ok((r.col("beds")?, r.col("occupied")?))
    }

    /// Money invoiced and collected on invoices finalised in the window. Amounts are summed as written, so
    /// this assumes one currency; a second currency in the window would be added into the same total.
    async fn money(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<(f64, f64, i64)> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
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
        Ok((r.col("invoiced")?, r.col("collected")?, r.col("n")?))
    }

    /// Check every reading and keep the alerts true to it: a measure that is red has one open alert (and the
    /// `ToleranceAlertRaised` event is published once, when it opens); an alert closes when its measure is
    /// amber or green. A measure with no data leaves its alert as it is, because nothing says it recovered.
    pub async fn evaluate(&self, days: i64) -> Result<Evaluation> {
        let status = self.status(days).await?;
        let readings: Vec<&Reading> = status
            .readings
            .iter()
            .chain(&status.by_service)
            .chain(&status.by_ward)
            .collect();
        let (mut raised, mut cleared) = (0, 0);
        for r in &readings {
            match (r.rag, r.value) {
                (Rag::Red, Some(value)) => {
                    if self.open_or_raise(r, value).await? {
                        raised += 1;
                    }
                }
                (Rag::Green | Rag::Amber, _) => {
                    let n = self
                        .db
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE tolerance_alerts SET cleared_at = now() \
                             WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3 AND cleared_at IS NULL",
                            [
                                r.metric.into(),
                                r.scope_kind.as_str().into(),
                                r.scope_key.clone().into(),
                            ],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .rows_affected();
                    cleared += n as usize;
                }
                _ => {}
            }
        }
        let open = self
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM tolerance_alerts WHERE cleared_at IS NULL"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?
            .and_then(|r| r.try_get::<i64>("", "n").ok())
            .unwrap_or(0);
        Ok(Evaluation {
            checked: readings.len(),
            raised,
            cleared,
            open,
        })
    }

    /// Open an alert for a red reading, or note that it is still red. True when a new alert opened.
    async fn open_or_raise(&self, r: &Reading, value: f64) -> Result<bool> {
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let id = Uuid::new_v4();
        let inserted = txn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO tolerance_alerts (id, metric, scope_kind, scope_key, value, target, tolerance, escalate_beyond) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING",
                [
                    id.into(),
                    r.metric.into(),
                    r.scope_kind.as_str().into(),
                    r.scope_key.clone().into(),
                    value.into(),
                    r.limits.target.into(),
                    r.limits.tolerance.into(),
                    r.limits.escalate_beyond.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?
            .rows_affected()
            == 1;
        if inserted {
            OutboxRepository::publish(
                &txn,
                "ToleranceAlertRaised",
                &serde_json::json!({
                    "alert_id": id,
                    "metric": r.metric,
                    "scope_kind": r.scope_kind,
                    "scope_key": r.scope_key,
                    "value": value,
                    "target": r.limits.target,
                    "escalate_beyond": r.limits.escalate_beyond,
                }),
            )
            .await?;
        } else {
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE tolerance_alerts SET last_seen_at = now(), value = $4 \
                 WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3 AND cleared_at IS NULL",
                [
                    r.metric.into(),
                    r.scope_kind.as_str().into(),
                    r.scope_key.clone().into(),
                    value.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        }
        txn.commit().await.map_err(Error::Database)?;
        Ok(inserted)
    }

    /// Alerts, newest first: the open ones, or every one (up to 200).
    pub async fn alerts(&self, open_only: bool) -> Result<Vec<Alert>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                format!(
                    "SELECT id, metric, scope_kind, scope_key, value, target, tolerance, escalate_beyond, raised_at, \
                            last_seen_at, cleared_at, acknowledged_by, acknowledged_at, note \
                     FROM tolerance_alerts {} ORDER BY raised_at DESC LIMIT 200",
                    if open_only { "WHERE cleared_at IS NULL" } else { "" }
                ),
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in &rows {
            let key: String = r.col("metric")?;
            let m = metric(&key);
            out.push(Alert {
                id: r.col("id")?,
                label: m.map(|m| m.label),
                unit: m.map(|m| m.unit),
                metric: key,
                scope_kind: r.col("scope_kind")?,
                scope_key: r.col("scope_key")?,
                value: r.col("value")?,
                target: r.col("target")?,
                tolerance: r.col("tolerance")?,
                escalate_beyond: r.col("escalate_beyond")?,
                raised_at: r.col("raised_at")?,
                last_seen_at: r.col("last_seen_at")?,
                cleared_at: r.col("cleared_at")?,
                acknowledged_by: r.col("acknowledged_by")?,
                acknowledged_at: r.col("acknowledged_at")?,
                note: r.col("note")?,
            });
        }
        Ok(out)
    }

    /// A named person says they are looking at an open alert. It stays open until the measure is no longer red.
    /// Once; audited.
    pub async fn acknowledge(
        &self,
        id: Uuid,
        by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = crate::validation::named_person(
            by,
            "name the person acknowledging the alert (acknowledged_by)",
        )?;
        let n = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE tolerance_alerts SET acknowledged_by = $2, acknowledged_at = now(), note = $3 \
                 WHERE id = $1 AND cleared_at IS NULL AND acknowledged_by IS NULL",
                [id.into(), by.into(), note.trim().into()],
            ))
            .await
            .map_err(Error::Database)?
            .rows_affected();
        if n == 0 {
            let exists = self
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT 1 AS x FROM tolerance_alerts WHERE id = $1",
                    [id.into()],
                ))
                .await
                .map_err(Error::Database)?
                .is_some();
            return Err(if exists {
                Error::conflict("the alert is closed or already acknowledged")
            } else {
                Error::not_found("no such alert")
            });
        }
        AuditLogRepository::log(
            &self.db,
            "tolerance_alert",
            id,
            "acknowledge",
            None,
            Some(serde_json::json!({ "acknowledged_by": by })),
            ctx,
        )
        .await
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
    fn a_scope_is_the_whole_service_one_service_or_one_ward() {
        assert_eq!(Scope::from_parts(None, None).unwrap(), Scope::all());
        assert_eq!(
            Scope::from_parts(Some(" "), Some("")).unwrap(),
            Scope::all()
        );
        let s = Scope::from_parts(Some(" cardiology "), None).unwrap();
        assert_eq!((s.kind, s.key.as_str()), (ScopeKind::Service, "cardiology"));
        let w = Scope::from_parts(None, Some("W1")).unwrap();
        assert_eq!(w.kind, ScopeKind::Ward);
        assert!(Scope::from_parts(Some("a"), Some("b")).is_err());
    }

    #[test]
    fn a_measure_narrows_only_the_way_it_is_read() {
        let svc = Scope::from_parts(Some("x"), None).unwrap();
        let ward = Scope::from_parts(None, Some("x")).unwrap();
        let get = |k| metric(k).unwrap();
        assert!(check_scope(get("wait_p90_weeks"), &svc).is_ok());
        assert!(check_scope(get("wait_p90_weeks"), &ward).is_err());
        assert!(check_scope(get("bed_occupancy"), &ward).is_ok());
        assert!(check_scope(get("bed_occupancy"), &svc).is_err());
        assert!(check_scope(get("invoice_collection_rate"), &svc).is_err());
        for m in METRICS {
            assert!(check_scope(&m, &Scope::all()).is_ok(), "{}", m.key);
        }
    }

    #[test]
    fn scopes_do_not_share_an_audit_id() {
        let a = Scope::all().uuid("slot_utilisation");
        let s = Scope::from_parts(Some("x"), None)
            .unwrap()
            .uuid("slot_utilisation");
        let t = Scope::from_parts(Some("y"), None)
            .unwrap()
            .uuid("slot_utilisation");
        assert!(a != s && s != t);
        assert_eq!(
            a,
            Uuid::new_v5(&Uuid::NAMESPACE_OID, b"slot_utilisation"),
            "unchanged for the whole service"
        );
    }

    #[test]
    fn the_worst_tier_wins_and_no_data_never_beats_green() {
        let worst = |tiers: &[Rag]| tiers.iter().copied().max_by_key(|r| r.rank());
        assert_eq!(worst(&[Rag::Green, Rag::Amber, Rag::Red]), Some(Rag::Red));
        assert_eq!(worst(&[Rag::Green, Rag::NoData]), Some(Rag::Green));
        assert_eq!(worst(&[Rag::NoData]), Some(Rag::NoData));
    }
}

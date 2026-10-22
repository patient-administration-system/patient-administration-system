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

mod alerts;
mod model;
mod settings;
mod status;
pub use model::*;

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

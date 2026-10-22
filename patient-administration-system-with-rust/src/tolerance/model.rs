//! Part of [`super`]: model.

use super::*;

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
    pub(super) fn rank(self) -> u8 {
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

    pub(super) fn parse(s: &str) -> Option<Self> {
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

    pub(super) fn uuid(&self, metric: &str) -> Uuid {
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

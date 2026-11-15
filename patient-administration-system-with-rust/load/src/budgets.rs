//! Latency and error budgets (NFR-29), read from `budgets.json`, and the check of a phase against them.

use crate::stats::OpSummary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Budget {
    pub p95_ms: f64,
    pub p99_ms: f64,
    /// Errors over calls, 0 to 1. A 409 on a contended slot is not an error.
    pub max_error_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Budgets {
    pub default: Budget,
    #[serde(default)]
    pub ops: BTreeMap<String, Budget>,
    /// Fewer calls than this in a phase are too few for a p99 to mean anything: the latency is not
    /// checked (the error rate still is).
    #[serde(default = "default_min_samples")]
    pub min_samples: u64,
}

fn default_min_samples() -> u64 {
    100
}

impl Budgets {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn for_op(&self, op: &str) -> Budget {
        self.ops.get(op).copied().unwrap_or(self.default)
    }
}

/// One budget broken.
#[derive(Debug, Clone, Serialize)]
pub struct Breach {
    pub phase: String,
    pub op: String,
    pub measure: &'static str,
    pub value: f64,
    pub limit: f64,
}

/// Check one phase's operations. `latency` is false for a phase where only errors count.
pub fn check(
    phase: &str,
    ops: &BTreeMap<String, OpSummary>,
    budgets: &Budgets,
    latency: bool,
) -> Vec<Breach> {
    let mut out = Vec::new();
    for (op, s) in ops {
        let b = budgets.for_op(op);
        let mut breach = |measure, value, limit| {
            out.push(Breach {
                phase: phase.to_string(),
                op: op.clone(),
                measure,
                value,
                limit,
            });
        };
        if s.error_rate > b.max_error_rate {
            breach("error_rate", s.error_rate, b.max_error_rate);
        }
        if latency && s.count >= budgets.min_samples {
            if s.p95_ms > b.p95_ms {
                breach("p95_ms", s.p95_ms, b.p95_ms);
            }
            if s.p99_ms > b.p99_ms {
                breach("p99_ms", s.p99_ms, b.p99_ms);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(count: u64, errors: u64, p95: f64, p99: f64) -> OpSummary {
        OpSummary {
            count,
            ok: count - errors,
            contended: 0,
            errors,
            error_rate: errors as f64 / count as f64,
            per_second: 0.0,
            mean_ms: 0.0,
            p50_ms: 0.0,
            p90_ms: 0.0,
            p95_ms: p95,
            p99_ms: p99,
            max_ms: p99,
            statuses: BTreeMap::new(),
        }
    }

    fn budgets() -> Budgets {
        Budgets::parse(
            r#"{"default": {"p95_ms": 50, "p99_ms": 100, "max_error_rate": 0.01},
                "ops": {"book_slot": {"p95_ms": 200, "p99_ms": 400, "max_error_rate": 0}}}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn an_op_uses_its_own_budget_or_the_default() {
        let b = budgets();
        assert_eq!(b.for_op("book_slot").p95_ms, 200.0);
        assert_eq!(b.for_op("get_slot").p95_ms, 50.0);
        assert_eq!(b.min_samples, 100);
    }

    #[test]
    fn each_measure_over_its_limit_is_one_breach() {
        let b = budgets();
        let mut ops = BTreeMap::new();
        ops.insert("get_slot".to_string(), summary(1000, 0, 60.0, 90.0));
        ops.insert("book_slot".to_string(), summary(1000, 1, 100.0, 500.0));
        let found = check("steady", &ops, &b, true);
        let names: Vec<_> = found.iter().map(|x| (x.op.as_str(), x.measure)).collect();
        assert_eq!(
            names,
            [
                ("book_slot", "error_rate"),
                ("book_slot", "p99_ms"),
                ("get_slot", "p95_ms")
            ]
        );
    }

    #[test]
    fn few_samples_or_an_errors_only_phase_skip_latency() {
        let b = budgets();
        let mut ops = BTreeMap::new();
        ops.insert("get_slot".to_string(), summary(10, 0, 999.0, 999.0));
        assert!(check("steady", &ops, &b, true).is_empty());
        ops.insert("get_slot".to_string(), summary(1000, 0, 999.0, 999.0));
        assert!(check("spike", &ops, &b, false).is_empty());
    }
}

//! What each operation did: every latency (exact percentiles, no histogram buckets), each status, and the
//! failures of the transport itself.

use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// How one call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The answer the operation expects (a 2xx, or a status the scenario names as normal, such as 409 on a
    /// slot another worker took first).
    Ok,
    /// Expected under contention and not an error, but counted apart: a 409 on a contended slot.
    Contended,
    /// Any other status, or no answer at all.
    Error,
}

/// One call's outcome.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub op: &'static str,
    /// `None` when there was no HTTP answer (refused, reset, timed out).
    pub status: Option<u16>,
    pub verdict: Verdict,
    pub elapsed: Duration,
}

#[derive(Debug, Default, Clone)]
pub struct OpStats {
    /// Microseconds, unsorted until summarised.
    latencies: Vec<u64>,
    statuses: BTreeMap<String, u64>,
    ok: u64,
    contended: u64,
    errors: u64,
}

/// Every operation's stats for one phase. Each worker keeps its own and they are merged at the end, so
/// recording takes no lock.
#[derive(Debug, Default, Clone)]
pub struct Recorder {
    ops: BTreeMap<&'static str, OpStats>,
    /// Calls the open-model generator could not start because the in-flight limit was reached. Each is a
    /// request a real client would have sent, so a non-zero count means the server fell behind.
    pub shed: u64,
}

impl Recorder {
    pub fn record(&mut self, s: Sample) {
        let e = self.ops.entry(s.op).or_default();
        e.latencies
            .push(u64::try_from(s.elapsed.as_micros()).unwrap_or(u64::MAX));
        let key = s
            .status
            .map_or_else(|| "none".to_string(), |c| c.to_string());
        *e.statuses.entry(key).or_default() += 1;
        match s.verdict {
            Verdict::Ok => e.ok += 1,
            Verdict::Contended => e.contended += 1,
            Verdict::Error => e.errors += 1,
        }
    }

    pub fn merge(&mut self, other: Recorder) {
        self.shed += other.shed;
        for (op, o) in other.ops {
            let e = self.ops.entry(op).or_default();
            e.latencies.extend(o.latencies);
            for (k, v) in o.statuses {
                *e.statuses.entry(k).or_default() += v;
            }
            e.ok += o.ok;
            e.contended += o.contended;
            e.errors += o.errors;
        }
    }

    pub fn saw_status(&self, code: u16) -> bool {
        let key = code.to_string();
        self.ops.values().any(|o| o.statuses.contains_key(&key))
    }

    pub fn summarise(&self, wall: Duration) -> BTreeMap<String, OpSummary> {
        self.ops
            .iter()
            .map(|(op, s)| ((*op).to_string(), OpSummary::of(s, wall)))
            .collect()
    }
}

/// One operation in one phase, as reported and as checked against its budget.
#[derive(Debug, Clone, Serialize)]
pub struct OpSummary {
    pub count: u64,
    pub ok: u64,
    pub contended: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub per_second: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub statuses: BTreeMap<String, u64>,
}

impl OpSummary {
    fn of(s: &OpStats, wall: Duration) -> Self {
        let mut l = s.latencies.clone();
        l.sort_unstable();
        let count = l.len() as u64;
        let ms = |us: u64| us as f64 / 1000.0;
        let mean = if l.is_empty() {
            0.0
        } else {
            ms(l.iter().sum::<u64>()) / l.len() as f64
        };
        Self {
            count,
            ok: s.ok,
            contended: s.contended,
            errors: s.errors,
            error_rate: if count == 0 {
                0.0
            } else {
                s.errors as f64 / count as f64
            },
            per_second: count as f64 / wall.as_secs_f64().max(f64::EPSILON),
            mean_ms: mean,
            p50_ms: ms(percentile(&l, 50.0)),
            p90_ms: ms(percentile(&l, 90.0)),
            p95_ms: ms(percentile(&l, 95.0)),
            p99_ms: ms(percentile(&l, 99.0)),
            max_ms: ms(l.last().copied().unwrap_or(0)),
            statuses: s.statuses.clone(),
        }
    }
}

/// The nearest-rank percentile of sorted values: the smallest value with at least `p` percent at or below it.
pub fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 50.0), 50);
        assert_eq!(percentile(&v, 95.0), 95);
        assert_eq!(percentile(&v, 99.0), 99);
        assert_eq!(percentile(&v, 100.0), 100);
        assert_eq!(percentile(&[7], 99.0), 7);
        assert_eq!(percentile(&[], 50.0), 0);
    }

    #[test]
    fn merging_adds_counts_and_keeps_every_latency() {
        let s = |op, status, verdict, ms| Sample {
            op,
            status,
            verdict,
            elapsed: Duration::from_millis(ms),
        };
        let mut a = Recorder::default();
        a.record(s("book", Some(200), Verdict::Ok, 10));
        a.record(s("book", Some(409), Verdict::Contended, 20));
        let mut b = Recorder::default();
        b.record(s("book", None, Verdict::Error, 30));
        b.shed = 2;
        a.merge(b);
        let sum = a.summarise(Duration::from_secs(1));
        let book = &sum["book"];
        assert_eq!(
            (book.count, book.ok, book.contended, book.errors),
            (3, 1, 1, 1)
        );
        assert!((book.error_rate - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(book.max_ms, 30.0);
        assert_eq!(book.statuses["none"], 1);
        assert_eq!(a.shed, 2);
        assert!(a.saw_status(409));
        assert!(!a.saw_status(429));
    }
}

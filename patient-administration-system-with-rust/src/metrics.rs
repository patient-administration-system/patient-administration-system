//! What an operator can see (QUAL-16): counters kept in the process and gauges read from the database when
//! asked, in the Prometheus text format.
//!
//! In process, since the server started: requests by status class and their total time, and for each
//! background job how many ticks ran, were skipped (another replica had the lock), found no connection, failed or
//! panicked, how many failed in a row, and when its work last succeeded. From the database at the moment of the request: how many outbox events wait to
//! be published and how long the oldest has waited, how many are dead-lettered, and how many staff call tasks
//! are open. No identifiers and no personal data. Served at `GET /api/admin/metrics` (administrators only).

use crate::db::sql::{Conn, Pool, Row, Stmt};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use jiff::Timestamp;

use crate::db::row::RowExt;
use crate::jobs::Outcome;
use crate::{Error, Result};

#[derive(Default, Clone, Copy)]
struct JobStats {
    ran: u64,
    skipped: u64,
    unavailable: u64,
    failed: u64,
    panicked: u64,
    /// Ticks in a row that ended unavailable, failed or panicked; a success sets it back to 0.
    failures_in_a_row: u32,
    /// Unix seconds of the last run that succeeded.
    last_success: Option<i64>,
    /// Unix seconds of the last tick that finished at all, including one skipped because another replica held
    /// the job's lock (that replica ran it).
    last_seen: Option<i64>,
    /// How often the job is meant to tick, once it is running in this process.
    every_secs: Option<u64>,
}

/// What `GET /api/health` says about a background job (NFR-34). There is no "ok" without data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobHealth {
    /// The job does not run in this process (a test, a tool).
    Off,
    /// It runs here but has not finished a tick yet.
    Unknown,
    /// It finished a tick within three of its intervals.
    Ok,
    /// It runs here and has not finished a tick for three intervals (and at least 30 seconds): stopped or
    /// stuck in one tick.
    Stalled,
    /// Its last [`FAILING_AFTER`] ticks or more all failed: they ended, but did not do the work.
    Failing,
}

/// Ticks in a row that must fail before a job's health is `failing`: one failure is often a passing fault.
pub const FAILING_AFTER: u32 = 3;

impl JobHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            JobHealth::Off => "off",
            JobHealth::Unknown => "unknown",
            JobHealth::Ok => "ok",
            JobHealth::Stalled => "stalled",
            JobHealth::Failing => "failing",
        }
    }
}

struct Registry {
    jobs: Mutex<BTreeMap<String, JobStats>>,
    /// When each job registered (Unix seconds), so one that never finishes a tick is seen as stalled.
    starts: Mutex<BTreeMap<String, i64>>,
    /// Requests by status class: 1xx, 2xx, 3xx, 4xx, 5xx.
    http: [AtomicU64; 5],
    http_micros: AtomicU64,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry {
        jobs: Mutex::new(BTreeMap::new()),
        starts: Mutex::new(BTreeMap::new()),
        http: Default::default(),
        http_micros: AtomicU64::new(0),
    })
}

/// When an HL7 v2 message last arrived, and when the outbox last delivered an event (Unix seconds; 0 is
/// never), for silence alarms (NFR-34, HUB-6). A deployment sets the thresholds, which differ by time of day
/// and week: a feed quiet at 3am may be normal, at 11am not.
static HL7_LAST_RECEIVED: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static OUTBOX_LAST_DELIVERED: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// An HL7 v2 message arrived (processed, refused or answered as a resend).
pub fn hl7_received() {
    HL7_LAST_RECEIVED.store(Timestamp::now().as_second(), Ordering::Relaxed);
}

/// The outbox delivered an event.
pub fn outbox_delivered() {
    OUTBOX_LAST_DELIVERED.store(Timestamp::now().as_second(), Ordering::Relaxed);
}

/// Seconds since an HL7 v2 message last arrived, and since the outbox last delivered, at `now`; `None` when it
/// has not happened since this process started.
pub fn silence_at(now: i64) -> (Option<i64>, Option<i64>) {
    let ago = |t: i64| {
        (t > 0)
            .then(|| now.checked_sub(t))
            .flatten()
            .map(|d| d.max(0))
    };
    (
        ago(HL7_LAST_RECEIVED.load(Ordering::Relaxed)),
        ago(OUTBOX_LAST_DELIVERED.load(Ordering::Relaxed)),
    )
}

/// A background job started in this process, to tick `every`. Until it does, its health is `Off`.
pub fn job_registered(name: &str, every: std::time::Duration) {
    let mut jobs = registry()
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let s = jobs.entry(name.to_string()).or_default();
    s.every_secs = Some(every.as_secs().max(1));
    s.last_seen = None;
    drop(jobs);
    registry()
        .starts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string(), Timestamp::now().as_second());
}

/// The health of job `name` at `now` (Unix seconds).
pub fn job_health_at(name: &str, now: i64) -> JobHealth {
    let jobs = registry()
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(s) = jobs.get(name).copied() else {
        return JobHealth::Off;
    };
    let Some(every) = s.every_secs else {
        return JobHealth::Off;
    };
    let window = every
        .checked_mul(3)
        .and_then(|w| i64::try_from(w).ok())
        .unwrap_or(i64::MAX)
        .max(30);
    drop(jobs);
    if s.failures_in_a_row >= FAILING_AFTER {
        return JobHealth::Failing;
    }
    match s.last_seen {
        Some(t) if now.checked_sub(t).is_some_and(|d| d <= window) => JobHealth::Ok,
        Some(_) => JobHealth::Stalled,
        None => {
            let started = registry()
                .starts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(name)
                .copied();
            match started {
                Some(t) if now.checked_sub(t).is_some_and(|d| d > window) => JobHealth::Stalled,
                _ => JobHealth::Unknown,
            }
        }
    }
}

/// The health of job `name` now.
pub fn job_health(name: &str) -> JobHealth {
    job_health_at(name, Timestamp::now().as_second())
}

/// A background job's tick ended.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: each counter here rises by one per item of a collection already in memory, or per event handled, so it cannot reach the maximum of its type"
)]
pub fn job_finished(name: &str, outcome: Outcome) {
    // A poisoned lock only means another thread panicked while counting; the counts are still usable.
    let mut jobs = registry()
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let s = jobs.entry(name.to_string()).or_default();
    let now = Timestamp::now().as_second();
    // Every tick that ended moves `last_seen`, so a failing job is not also reported stalled; only a success
    // moves `last_success`.
    s.last_seen = Some(now);
    match outcome {
        Outcome::Ran => {
            s.ran += 1;
            s.last_success = Some(now);
            s.failures_in_a_row = 0;
        }
        // The replica that held the lock ran it; this one has nothing to report either way.
        Outcome::Skipped => s.skipped += 1,
        Outcome::Unavailable | Outcome::Failed | Outcome::Panicked => {
            match outcome {
                Outcome::Unavailable => s.unavailable += 1,
                Outcome::Failed => s.failed += 1,
                _ => s.panicked += 1,
            }
            // A count at the maximum stays there: it is only compared with FAILING_AFTER.
            s.failures_in_a_row = s.failures_in_a_row.checked_add(1).unwrap_or(u32::MAX);
        }
    }
}

/// A request was answered with `status` after `micros` microseconds.
pub fn http_finished(status: u16, micros: u64) {
    let r = registry();
    // Classes 1xx to 5xx are counted; a status outside them is not a class, so it counts in none.
    if let Some(counter) = usize::from(status / 100)
        .checked_sub(1)
        .and_then(|class| r.http.get(class))
    {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    r.http_micros.fetch_add(micros, Ordering::Relaxed);
}

/// The process-side numbers as Prometheus text.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: `i` indexes the five status classes, so `i + 1` is at most 5"
)]
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "RCA-CONV-4: the microseconds counter would need 285 years to pass 2^53"
)]
fn render_process() -> String {
    let r = registry();
    let mut out = String::new();
    out.push_str(
        "# HELP pas_http_requests_total Requests answered since start, by status class.\n",
    );
    out.push_str("# TYPE pas_http_requests_total counter\n");
    for (i, c) in r.http.iter().enumerate() {
        let _ = writeln!(
            out,
            "pas_http_requests_total{{class=\"{}xx\"}} {}",
            i + 1,
            c.load(Ordering::Relaxed)
        );
    }
    out.push_str(
        "# HELP pas_http_request_seconds_total Time spent answering requests since start.\n",
    );
    out.push_str("# TYPE pas_http_request_seconds_total counter\n");
    let _ = writeln!(
        out,
        "pas_http_request_seconds_total {:.6}",
        r.http_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0
    );
    let jobs: Vec<(String, JobStats)> = r
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    out.push_str("# HELP pas_job_runs_total Background job ticks since start, by outcome.\n");
    out.push_str("# TYPE pas_job_runs_total counter\n");
    for (name, s) in &jobs {
        for (outcome, n) in [
            ("ran", s.ran),
            ("skipped", s.skipped),
            ("unavailable", s.unavailable),
            ("failed", s.failed),
            ("panicked", s.panicked),
        ] {
            let _ = writeln!(
                out,
                "pas_job_runs_total{{job=\"{name}\",outcome=\"{outcome}\"}} {n}"
            );
        }
    }
    out.push_str(
        "# HELP pas_job_last_success_timestamp_seconds When the job's work last succeeded (Unix seconds).\n",
    );
    out.push_str("# TYPE pas_job_last_success_timestamp_seconds gauge\n");
    for (name, s) in &jobs {
        if let Some(t) = s.last_success {
            let _ = writeln!(
                out,
                "pas_job_last_success_timestamp_seconds{{job=\"{name}\"}} {t}"
            );
        }
    }
    out.push_str(
        "# HELP pas_job_failures_in_a_row Ticks in a row that were unavailable, failed or panicked; 0 after a success.\n",
    );
    out.push_str("# TYPE pas_job_failures_in_a_row gauge\n");
    for (name, s) in &jobs {
        let _ = writeln!(
            out,
            "pas_job_failures_in_a_row{{job=\"{name}\"}} {}",
            s.failures_in_a_row
        );
    }
    for (name, help, cell) in [
        (
            "pas_hl7_last_received_timestamp_seconds",
            "When an HL7 v2 message last arrived (Unix seconds); absent until one has.",
            &HL7_LAST_RECEIVED,
        ),
        (
            "pas_outbox_last_delivered_timestamp_seconds",
            "When the outbox last delivered an event (Unix seconds); absent until it has.",
            &OUTBOX_LAST_DELIVERED,
        ),
    ] {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} gauge");
        let t = cell.load(Ordering::Relaxed);
        if t > 0 {
            let _ = writeln!(out, "{name} {t}");
        }
    }
    out
}

async fn one_row(db: &Pool, sql: &str) -> Result<Row> {
    db.one(Stmt::text(sql.to_string()))
        .await?
        .ok_or_else(|| Error::internal("a count returned no row"))
}

/// Everything, as Prometheus text: the process counters and the database gauges.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "RCA-CONV-4: each figure is a count of queued rows, far below 2^53"
)]
pub async fn render(db: &Pool) -> Result<String> {
    let mut out = render_process();
    let outbox = one_row(
        db,
        "SELECT count(*)::bigint AS n, COALESCE(extract(epoch FROM now() - min(at)), 0)::float8 AS oldest \
         FROM outbox_events WHERE NOT published",
    )
    .await?;
    let dead = one_row(db, "SELECT count(*)::bigint AS n FROM outbox_dead_letters").await?;
    let calls = one_row(
        db,
        "SELECT count(*)::bigint AS n FROM staff_call_tasks WHERE handled_at IS NULL",
    )
    .await?;
    let gauges = [
        (
            "pas_outbox_pending",
            "Outbox events waiting to be published.",
            outbox.col::<i64>("n")? as f64,
        ),
        (
            "pas_outbox_oldest_pending_seconds",
            "How long the oldest unpublished outbox event has waited.",
            outbox.col::<f64>("oldest")?,
        ),
        (
            "pas_outbox_dead_letters",
            "Outbox events given up on and waiting for a person.",
            dead.col::<i64>("n")? as f64,
        ),
        (
            "pas_staff_call_tasks_open",
            "Staff call tasks not yet handled.",
            calls.col::<i64>("n")? as f64,
        ),
    ];
    for (name, help, value) in gauges {
        let _ = writeln!(
            out,
            "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}"
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_unknown_until_it_happens_then_counted_from_it() {
        hl7_received();
        outbox_delivered();
        let now = Timestamp::now().as_second();
        let (hl7, outbox) = silence_at(now + 90);
        assert!((89..=91).contains(&hl7.unwrap()), "{hl7:?}");
        assert!((89..=91).contains(&outbox.unwrap()), "{outbox:?}");
        let text = render_process();
        assert!(
            text.contains("pas_hl7_last_received_timestamp_seconds "),
            "{text}"
        );
        assert!(
            text.contains("pas_outbox_last_delivered_timestamp_seconds "),
            "{text}"
        );
    }

    #[test]
    fn job_health_is_off_unknown_ok_or_stalled_and_never_ok_without_data() {
        let now = Timestamp::now().as_second();
        assert_eq!(job_health_at("hub6-never-registered", now), JobHealth::Off);
        job_registered("hub6-job", std::time::Duration::from_secs(2));
        assert_eq!(job_health_at("hub6-job", now), JobHealth::Unknown);
        // Registered and never finished a tick for longer than the window (30 s at least): stalled.
        assert_eq!(job_health_at("hub6-job", now + 31), JobHealth::Stalled);
        job_finished("hub6-job", Outcome::Ran);
        assert_eq!(job_health_at("hub6-job", now + 1), JobHealth::Ok);
        assert_eq!(job_health_at("hub6-job", now + 31), JobHealth::Stalled);
        // A tick another replica ran counts: the job is alive somewhere.
        job_finished("hub6-job", Outcome::Skipped);
        assert_eq!(job_health_at("hub6-job", now + 2), JobHealth::Ok);
        // A slow job's window is three of its intervals.
        job_registered("hub6-hourly", std::time::Duration::from_secs(3600));
        job_finished("hub6-hourly", Outcome::Ran);
        assert_eq!(
            job_health_at("hub6-hourly", now + 3 * 3600 - 5),
            JobHealth::Ok
        );
    }

    #[test]
    fn job_runs_are_counted_by_outcome_and_the_last_success_is_stamped() {
        job_finished("unit-test-job", Outcome::Skipped);
        job_finished("unit-test-job", Outcome::Ran);
        job_finished("unit-test-job", Outcome::Panicked);
        let text = render_process();
        assert!(
            text.contains("pas_job_runs_total{job=\"unit-test-job\",outcome=\"ran\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("pas_job_runs_total{job=\"unit-test-job\",outcome=\"skipped\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("pas_job_runs_total{job=\"unit-test-job\",outcome=\"panicked\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("pas_job_last_success_timestamp_seconds{job=\"unit-test-job\"}"),
            "{text}"
        );
    }

    #[test]
    fn requests_are_counted_by_status_class() {
        let before = render_process();
        http_finished(200, 1500);
        http_finished(404, 500);
        http_finished(503, 10);
        let after = render_process();
        let count = |text: &str, class: &str| -> u64 {
            text.lines()
                .find(|l| l.starts_with(&format!("pas_http_requests_total{{class=\"{class}\"}}")))
                .and_then(|l| l.rsplit(' ').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        assert!(count(&after, "2xx") > count(&before, "2xx"));
        assert!(count(&after, "4xx") > count(&before, "4xx"));
        assert!(count(&after, "5xx") > count(&before, "5xx"));
    }

    #[test]
    fn a_status_outside_the_classes_is_counted_in_none_and_does_not_panic() {
        http_finished(0, 0);
        http_finished(999, 0);
    }

    /// PA-10: errors are not successes. A job whose ticks fail keeps its last success where it was, is failing
    /// after `FAILING_AFTER` in a row, and is well again after one success; a tick another replica ran is
    /// neither, and a missing connection is counted apart from an error.
    #[test]
    fn failed_ticks_do_not_count_as_success_and_enough_in_a_row_are_failing() {
        let name = "pa10-job";
        let now = Timestamp::now().as_second();
        job_registered(name, std::time::Duration::from_secs(60));
        let last_success = || {
            render_process()
                .lines()
                .find(|l| {
                    l.starts_with(&format!(
                        "pas_job_last_success_timestamp_seconds{{job=\"{name}\"}}"
                    ))
                })
                .map(String::from)
        };
        job_finished(name, Outcome::Failed);
        job_finished(name, Outcome::Unavailable);
        assert_eq!(
            job_health_at(name, now),
            JobHealth::Ok,
            "two failures may pass"
        );
        assert_eq!(last_success(), None, "no success yet");
        job_finished(name, Outcome::Skipped);
        job_finished(name, Outcome::Failed);
        assert_eq!(job_health_at(name, now), JobHealth::Failing);
        let text = render_process();
        for line in [
            format!("pas_job_runs_total{{job=\"{name}\",outcome=\"failed\"}} 2"),
            format!("pas_job_runs_total{{job=\"{name}\",outcome=\"unavailable\"}} 1"),
            format!("pas_job_runs_total{{job=\"{name}\",outcome=\"skipped\"}} 1"),
            format!("pas_job_failures_in_a_row{{job=\"{name}\"}} 3"),
        ] {
            assert!(text.contains(&line), "{line} in {text}");
        }
        job_finished(name, Outcome::Ran);
        assert_eq!(job_health_at(name, now), JobHealth::Ok);
        assert!(last_success().is_some());
        assert!(
            render_process().contains(&format!("pas_job_failures_in_a_row{{job=\"{name}\"}} 0"))
        );
    }
}

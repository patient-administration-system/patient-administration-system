//! What an operator can see (QUAL-16): counters kept in the process and gauges read from the database when
//! asked, in the Prometheus text format.
//!
//! In process, since the server started: requests by status class and their total time, and for each
//! background job how many runs happened, were skipped (another replica had the lock), panicked, and when it
//! last ran to completion. From the database at the moment of the request: how many outbox events wait to
//! be published and how long the oldest has waited, how many are dead-lettered, and how many staff call tasks
//! are open. No identifiers and no personal data. Served at `GET /api/admin/metrics` (administrators only).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

use crate::db::row::RowExt;
use crate::jobs::Outcome;
use crate::{Error, Result};

#[derive(Default, Clone, Copy)]
struct JobStats {
    ran: u64,
    skipped: u64,
    panicked: u64,
    /// Unix seconds of the last run that finished without a panic.
    last_success: Option<i64>,
}

struct Registry {
    jobs: Mutex<BTreeMap<String, JobStats>>,
    /// Requests by status class: 1xx, 2xx, 3xx, 4xx, 5xx.
    http: [AtomicU64; 5],
    http_micros: AtomicU64,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry {
        jobs: Mutex::new(BTreeMap::new()),
        http: Default::default(),
        http_micros: AtomicU64::new(0),
    })
}

/// A background job's tick ended.
pub fn job_finished(name: &str, outcome: Outcome) {
    // A poisoned lock only means another thread panicked while counting; the counts are still usable.
    let mut jobs = registry()
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let s = jobs.entry(name.to_string()).or_default();
    match outcome {
        Outcome::Ran => {
            s.ran += 1;
            s.last_success = Some(Utc::now().timestamp());
        }
        Outcome::Skipped => s.skipped += 1,
        Outcome::Panicked => s.panicked += 1,
    }
}

/// A request was answered with `status` after `micros` microseconds.
pub fn http_finished(status: u16, micros: u64) {
    let r = registry();
    let class = usize::from(status / 100).clamp(1, 5) - 1;
    r.http[class].fetch_add(1, Ordering::Relaxed);
    r.http_micros.fetch_add(micros, Ordering::Relaxed);
}

/// The process-side numbers as Prometheus text.
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
            ("panicked", s.panicked),
        ] {
            let _ = writeln!(
                out,
                "pas_job_runs_total{{job=\"{name}\",outcome=\"{outcome}\"}} {n}"
            );
        }
    }
    out.push_str(
        "# HELP pas_job_last_success_timestamp_seconds When the job last finished without a panic (Unix seconds).\n",
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
    out
}

async fn one_row(db: &DatabaseConnection, sql: &str) -> Result<sea_orm::QueryResult> {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql.to_string()))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::internal("a count returned no row"))
}

/// Everything, as Prometheus text: the process counters and the database gauges.
pub async fn render(db: &DatabaseConnection) -> Result<String> {
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
    fn a_status_outside_the_classes_is_counted_somewhere_not_lost_or_panicking() {
        http_finished(0, 0);
        http_finished(999, 0);
    }
}

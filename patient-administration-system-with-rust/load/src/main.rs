//! `pas-load` — load tests for the Patient Administration System (PERF-1, NFR-29).
//!
//! It makes its own rows in the database (`DATABASE_URL`), then drives a **running** server over HTTP
//! (`--url`), so every layer a caller meets is measured: the network, compression, the request id and
//! rate-limit layers, authentication, the handler, the database. It reports each operation's throughput,
//! latency percentiles and errors per phase, checks them against `budgets.json`, and checks that the
//! answers stayed right under load: no slot booked twice when many callers race for it, and an audit row
//! and an outbox event for every booking (NFR-1, NFR-2).
//!
//! ```text
//! pas-load [--url URL] [--profile smoke|load|stress|spike|soak] [--scenario read|cycle|contention|ihe|mixed|all]
//!          [--workers N] [--duration 30s|5m] [--rate PER_SECOND] [--budgets FILE] [--report FILE] [--token T]
//! ```
//!
//! Run the server under test with `PAS_RATE_LIMIT_RPM=0` (one client sending thousands of requests a minute
//! is what the limit exists to stop); a 429 makes the run fail, because the numbers would measure the limit.
//! Exit status: 0 when every check passed, 1 when a budget or a correctness check failed, 2 when the run
//! could not start.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod budgets;
mod fixtures;
mod runner;
mod scenarios;
mod stats;

use budgets::{Breach, Budgets};
use fixtures::{Db, Fail, Sizes};
use runner::{Check, Overrides, Phase};
use scenarios::{Ctx, Scenario};
use serde::Serialize;
use stats::{OpSummary, Recorder};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
struct Args {
    url: String,
    database_url: String,
    profile: String,
    scenarios: Vec<Scenario>,
    overrides: Overrides,
    budgets: String,
    report: Option<String>,
    token: Option<String>,
}

fn duration(s: &str) -> Option<Duration> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().ok()?;
    Some(Duration::from_secs(match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return None,
    }))
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        url: env("PAS_LOAD_URL").unwrap_or_else(|| "http://127.0.0.1:8080".into()),
        // RCA-DECL-1: absent DATABASE_URL is blank, and the profiles that need a database refuse it.
        database_url: env("DATABASE_URL").unwrap_or(String::new()),
        profile: "smoke".into(),
        scenarios: Scenario::ALL.to_vec(),
        overrides: Overrides::default(),
        budgets: format!("{}/budgets.json", env!("CARGO_MANIFEST_DIR")),
        report: None,
        token: env("PAS_LOAD_TOKEN"),
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--url" => a.url = value()?.trim_end_matches('/').to_string(),
            "--database-url" => a.database_url = value()?,
            "--profile" => a.profile = value()?,
            "--scenario" => {
                let v = value()?;
                a.scenarios = if v == "all" {
                    Scenario::ALL.to_vec()
                } else {
                    v.split(',')
                        .map(|s| Scenario::parse(s).ok_or(format!("unknown scenario {s:?}")))
                        .collect::<Result<_, _>>()?
                };
            }
            "--workers" => {
                a.overrides.workers = Some(value()?.parse().map_err(|_| "--workers is a number")?);
            }
            "--duration" => {
                a.overrides.duration =
                    Some(duration(&value()?).ok_or("--duration is like 30s, 5m or 1h")?);
            }
            "--rate" => {
                a.overrides.rate = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--rate is requests per second")?,
                );
            }
            "--budgets" => a.budgets = value()?,
            "--report" => a.report = Some(value()?),
            "--token" => a.token = Some(value()?),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    if a.database_url.is_empty() {
        return Err("DATABASE_URL (or --database-url) is required: the fixtures are made in the database the server uses".into());
    }
    Ok(a)
}

const USAGE: &str = "pas-load [--url URL] [--profile smoke|load|stress|spike|soak] \
[--scenario read|cycle|contention|ihe|mixed|all] [--workers N] [--duration 30s|5m|1h] [--rate PER_SECOND] \
[--budgets FILE] [--report FILE] [--token T]";

#[derive(Debug, Serialize)]
struct PhaseReport {
    scenario: &'static str,
    phase: String,
    workers: usize,
    rate: Option<f64>,
    seconds: f64,
    shed: u64,
    ops: BTreeMap<String, OpSummary>,
}

#[derive(Debug, Serialize)]
struct Correctness {
    name: &'static str,
    passed: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Report {
    run_id: String,
    url: String,
    profile: String,
    started_at: String,
    phases: Vec<PhaseReport>,
    breaches: Vec<Breach>,
    /// For the stress profile: the first step, per scenario, that broke a budget.
    knees: BTreeMap<&'static str, String>,
    correctness: Vec<Correctness>,
    passed: bool,
}

fn print_phase(p: &PhaseReport) {
    let mode = p.rate.map_or_else(
        || format!("{} workers", p.workers),
        |r| format!("{r}/s, ≤{} in flight", p.workers),
    );
    println!(
        "\n{} / {} ({mode}, {:.0}s{})",
        p.scenario,
        p.phase,
        p.seconds,
        if p.shed > 0 {
            format!(", {} shed", p.shed)
        } else {
            String::new()
        }
    );
    println!(
        "  {:<24} {:>8} {:>8} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "operation", "count", "per s", "err %", "p50 ms", "p95 ms", "p99 ms", "max ms", "409"
    );
    for (op, s) in &p.ops {
        println!(
            "  {op:<24} {:>8} {:>8.1} {:>7.2} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8}",
            s.count,
            s.per_second,
            s.error_rate * 100.0,
            s.p50_ms,
            s.p95_ms,
            s.p99_ms,
            s.max_ms,
            s.contended
        );
    }
}

/// Soak: each operation's p95 in the last window must be within twice the first window's, plus 5 ms
/// (a floor, so that 1 ms becoming 3 ms is not called drift).
fn soak_drift(phases: &[&PhaseReport]) -> Vec<Breach> {
    let mut out = Vec::new();
    let windows: Vec<_> = phases
        .iter()
        .filter(|p| p.phase.starts_with("minute-"))
        .collect();
    let (Some(first), Some(last)) = (windows.first(), windows.last()) else {
        return out;
    };
    if windows.len() < 2 {
        return out;
    }
    for (op, l) in &last.ops {
        if let Some(f) = first.ops.get(op) {
            let limit = f.p95_ms * 2.0 + 5.0;
            if l.p95_ms > limit {
                out.push(Breach {
                    phase: format!("{} vs {}", last.phase, first.phase),
                    op: op.clone(),
                    measure: "p95_drift_ms",
                    value: l.p95_ms,
                    limit,
                });
            }
        }
    }
    out
}

async fn correctness(db: &Db, ctx: &Ctx, saw_429: bool) -> Result<Vec<Correctness>, Fail> {
    let mut out = vec![Correctness {
        name: "not rate limited",
        passed: !saw_429,
        detail: if saw_429 {
            "the server answered 429: run it with PAS_RATE_LIMIT_RPM=0, or the numbers measure the limit".into()
        } else {
            "no 429".into()
        },
    }];
    let wins = ctx
        .contention_wins
        .lock()
        .map(|w| w.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
    if !wins.is_empty() {
        let holders = db.holders_per_slot(&ctx.fx.hot_slots).await?;
        let doubled: Vec<_> = holders.iter().filter(|(_, n)| *n > 1).collect();
        let won_twice = wins.values().filter(|v| v.len() > 1).count();
        let held: i64 = holders.iter().fold(0_i64, |total, (_, n)| {
            total.checked_add(*n).unwrap_or(i64::MAX)
        });
        let won: usize = wins.values().fold(0_usize, |total, v| {
            total.checked_add(v.len()).unwrap_or(usize::MAX)
        });
        out.push(Correctness {
            name: "each contended slot went to one caller",
            passed: doubled.is_empty() && won_twice == 0 && held == i64::try_from(won)?,
            detail: format!(
                "{} hot slots, {won} won over HTTP, {held} held in the database, {} held twice, {won_twice} won twice",
                holders.len(),
                doubled.len()
            ),
        });
    }
    let booked = ctx
        .booked
        .lock()
        .map(|b| b.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
    if !booked.is_empty() {
        let (audit, outbox) = db.audit_and_outbox(&booked).await?;
        let n = i64::try_from(booked.len())?;
        out.push(Correctness {
            name: "every booking has its audit row and outbox event",
            passed: audit == n && outbox == n,
            detail: format!("{n} booked over HTTP, {audit} with a book audit row, {outbox} with an AppointmentBooked event"),
        });
    }
    Ok(out)
}

async fn run_scenario(
    ctx: &Arc<Ctx>,
    scenario: Scenario,
    phases: &[Phase],
    budgets: &Budgets,
    report: &mut Report,
) -> Recorder {
    let mut seen = Recorder::default();
    for phase in phases {
        let (rec, wall) = runner::run_phase(ctx, scenario, phase).await;
        let p = PhaseReport {
            scenario: scenario.name(),
            phase: phase.name.clone(),
            workers: phase.workers,
            rate: phase.rate,
            seconds: wall.as_secs_f64(),
            shed: rec.shed,
            ops: rec.summarise(wall),
        };
        print_phase(&p);
        if phase.check != Check::None {
            let breaches = budgets::check(&p.phase, &p.ops, budgets, phase.check == Check::Budgets);
            if report.profile == "stress" && !breaches.is_empty() {
                report
                    .knees
                    .entry(scenario.name())
                    .or_insert_with(|| p.phase.clone());
            } else {
                report.breaches.extend(breaches.into_iter().map(|mut b| {
                    b.phase = format!("{}/{}", scenario.name(), b.phase);
                    b
                }));
            }
        }
        seen.merge(rec);
        report.phases.push(p);
    }
    if report.profile == "soak" {
        let mine: Vec<&PhaseReport> = report
            .phases
            .iter()
            .filter(|p| p.scenario == scenario.name())
            .collect();
        let drift = soak_drift(&mine);
        report.breaches.extend(drift);
    }
    seen
}

async fn run(args: Args) -> Result<bool, Fail> {
    run_with_sizes(
        args,
        Sizes {
            read: 400,
            cycle: 2000,
            hot: 20,
            ihe: 2000,
        },
    )
    .await
}

async fn run_with_sizes(args: Args, sizes: Sizes) -> Result<bool, Fail> {
    let budgets = Budgets::parse(&std::fs::read_to_string(&args.budgets)?)?;
    let phases = runner::profile(&args.profile, args.overrides)
        .ok_or_else(|| format!("unknown profile {:?}", args.profile))?;
    // reqwest is built without a bundled TLS provider: install ring before the client.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(512)
        .build()?;
    let health = client.get(format!("{}/api/health", args.url)).send().await;
    match health {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => return Err(format!("{}/api/health answered {}", args.url, r.status()).into()),
        Err(e) => return Err(format!("no server at {}: {e}", args.url).into()),
    }
    let run_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    println!(
        "pas-load run {run_id}: profile {} against {}",
        args.profile, args.url
    );
    let db = Db::open(&args.database_url).await?;
    let fx = db.make(&run_id, sizes).await?;
    println!("fixtures made: schedules named load-{run_id}-*");
    let ctx = Arc::new(Ctx::new(client, args.url.clone(), args.token.clone(), fx));
    let mut report = Report {
        run_id,
        url: args.url.clone(),
        profile: args.profile.clone(),
        started_at: jiff::Timestamp::now().to_string(),
        phases: Vec::new(),
        breaches: Vec::new(),
        knees: BTreeMap::new(),
        correctness: Vec::new(),
        passed: false,
    };
    let mut seen = Recorder::default();
    for &s in &args.scenarios {
        seen.merge(run_scenario(&ctx, s, &phases, &budgets, &mut report).await);
    }
    report.correctness = correctness(&db, &ctx, seen.saw_status(429)).await?;
    report.passed = report.breaches.is_empty() && report.correctness.iter().all(|c| c.passed);
    print_verdict(&report);
    if let Some(path) = &args.report {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        println!("report written to {path}");
    }
    Ok(report.passed)
}

fn print_verdict(r: &Report) {
    println!();
    for c in &r.correctness {
        println!(
            "{} {}: {}",
            if c.passed { "ok  " } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    for b in &r.breaches {
        println!(
            "FAIL budget {} {} {}: {:.2} over {:.2}",
            b.phase, b.op, b.measure, b.value, b.limit
        );
    }
    for (s, k) in &r.knees {
        println!("knee {s}: the first stress step over budget was {k}");
    }
    println!("{}", if r.passed { "PASSED" } else { "FAILED" });
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(1),
        Err(e) => {
            eprintln!("pas-load: {e}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Fixtures;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    async fn run_server(status: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind run test server");
        let address = listener.local_addr().expect("run test server address");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let _read = socket.read(&mut request).await;
                    let body = "{}";
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _written = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (format!("http://{address}"), server)
    }

    fn run_args(url: String, database_url: String) -> Args {
        Args {
            url,
            database_url,
            profile: "smoke".into(),
            scenarios: vec![Scenario::Read],
            overrides: Overrides {
                workers: Some(1),
                duration: Some(Duration::from_millis(25)),
                rate: None,
            },
            budgets: format!("{}/budgets.json", env!("CARGO_MANIFEST_DIR")),
            report: None,
            token: None,
        }
    }

    fn report(profile: &str) -> Report {
        Report {
            run_id: "test".into(),
            url: "http://example.test".into(),
            profile: profile.into(),
            started_at: "2026-11-20T00:00:00Z".into(),
            phases: Vec::new(),
            breaches: Vec::new(),
            knees: BTreeMap::new(),
            correctness: Vec::new(),
            passed: false,
        }
    }

    fn run_context(base: String) -> Arc<Ctx> {
        let now = jiff::Timestamp::now();
        Arc::new(Ctx::new(
            reqwest::Client::new(),
            base,
            None,
            Fixtures {
                read_slots: vec![Uuid::new_v4()],
                read_appointments: vec![Uuid::new_v4()],
                read_day: now,
                cycle_slots: vec![Uuid::new_v4()],
                hot_slots: vec![Uuid::new_v4()],
                ihe_practitioner: Uuid::new_v4(),
                ihe_window: (now, now),
            },
        ))
    }

    fn args(v: &[&str]) -> Result<Args, String> {
        let mut all = vec!["--database-url", "postgres://x"];
        all.extend_from_slice(v);
        parse_args(all.into_iter().map(String::from))
    }

    #[test]
    fn durations_read_seconds_minutes_and_hours() {
        assert_eq!(duration("45"), Some(Duration::from_secs(45)));
        assert_eq!(duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(duration("5d"), None);
        assert_eq!(duration("m"), None);
    }

    #[test]
    fn options_are_read_and_bad_ones_refused() {
        let a = args(&[
            "--url",
            "http://h:1/",
            "--scenario",
            "read,ihe",
            "--workers",
            "7",
            "--rate",
            "20",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.url, "http://h:1");
        assert_eq!(a.scenarios, [Scenario::Read, Scenario::Ihe]);
        assert_eq!(
            (a.overrides.workers, a.overrides.rate),
            (Some(7), Some(20.0))
        );
        assert!(args(&["--scenario", "nope"]).is_err());
        assert!(args(&["--workers"]).is_err());
        assert!(args(&["--frobnicate"]).is_err());
    }

    #[test]
    fn every_command_line_value_and_error_is_exercised() {
        let a = args(&[
            "--profile",
            "soak",
            "--scenario",
            "all",
            "--duration",
            "2m",
            "--budgets",
            "limits.json",
            "--report",
            "report.json",
            "--token",
            "secret",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.profile, "soak");
        assert_eq!(a.scenarios, Scenario::ALL);
        assert_eq!(a.overrides.duration, Some(Duration::from_secs(120)));
        assert_eq!(a.budgets, "limits.json");
        assert_eq!(a.report.as_deref(), Some("report.json"));
        assert_eq!(a.token.as_deref(), Some("secret"));

        for bad in [
            vec!["--workers", "many"],
            vec!["--duration", "tomorrow"],
            vec!["--rate", "fast"],
            vec!["--help"],
            vec!["-h"],
        ] {
            assert!(args(&bad).is_err(), "{bad:?}");
        }
        assert!(
            parse_args(["--database-url", ""].into_iter().map(String::from)).is_err(),
            "an explicitly empty database URL must be refused even when the environment sets one"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn correctness_compares_http_wins_with_database_evidence() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "DATABASE_URL not set; skipping correctness_compares_http_wins_with_database_evidence"
            );
            return;
        };
        let db = Db::open(&url).await.expect("open correctness database");
        let run = Uuid::new_v4().simple().to_string();
        let mut fx = db
            .make(
                &run,
                Sizes {
                    read: 2,
                    cycle: 0,
                    hot: 0,
                    ihe: 0,
                },
            )
            .await
            .expect("make correctness fixtures");
        let slot = fx.read_slots[0];
        let appointment = fx.read_appointments[0];
        fx.hot_slots = vec![slot];
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let ctx = Ctx::new(reqwest::Client::new(), "http://unused".into(), None, fx);
        ctx.contention_wins
            .lock()
            .expect("contention wins lock")
            .insert(slot, vec![appointment]);
        ctx.booked.lock().expect("booked lock").push(appointment);

        let checks = correctness(&db, &ctx, false)
            .await
            .expect("successful correctness checks");
        assert_eq!(checks.len(), 3);
        assert!(checks.iter().all(|check| check.passed), "{checks:?}");

        ctx.contention_wins
            .lock()
            .expect("contention wins lock")
            .insert(slot, vec![appointment, Uuid::new_v4()]);
        let failed = correctness(&db, &ctx, true)
            .await
            .expect("failed correctness checks are reported");
        assert!(!failed[0].passed, "a 429 fails the run");
        assert!(!failed[1].passed, "two HTTP wins for one slot fail the run");
        assert!(failed[2].passed, "the real booking still has its evidence");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn a_complete_small_run_writes_a_passing_report_and_startup_errors_are_clear() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "DATABASE_URL not set; skipping a_complete_small_run_writes_a_passing_report_and_startup_errors_are_clear"
            );
            return;
        };
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let (base, server) = run_server("200 OK").await;
        let report_path = std::env::temp_dir().join(format!(
            "pas-load-small-run-{}.json",
            Uuid::new_v4().simple()
        ));
        let mut args = run_args(base, database_url.clone());
        args.report = Some(report_path.to_string_lossy().into_owned());
        assert!(
            run_with_sizes(
                args,
                Sizes {
                    read: 2,
                    cycle: 0,
                    hot: 0,
                    ihe: 0,
                },
            )
            .await
            .expect("small load run")
        );
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report_path).expect("read small-run report"))
                .expect("parse small-run report");
        assert_eq!(report["passed"], true);
        assert_eq!(report["profile"], "smoke");
        std::fs::remove_file(report_path).expect("remove small-run report");
        server.abort();

        let mut bad_profile = run_args("http://127.0.0.1:9".into(), database_url.clone());
        bad_profile.profile = "unknown".into();
        assert!(
            run_with_sizes(
                bad_profile,
                Sizes {
                    read: 0,
                    cycle: 0,
                    hot: 0,
                    ihe: 0
                }
            )
            .await
            .expect_err("unknown profile must fail")
            .to_string()
            .contains("unknown profile")
        );

        let mut missing_budgets = run_args("http://127.0.0.1:9".into(), database_url.clone());
        missing_budgets.budgets = "/file-that-does-not-exist/pas-budgets.json".into();
        assert!(
            run_with_sizes(
                missing_budgets,
                Sizes {
                    read: 0,
                    cycle: 0,
                    hot: 0,
                    ihe: 0
                }
            )
            .await
            .is_err()
        );

        let (unhealthy, unhealthy_server) = run_server("503 Service Unavailable").await;
        let error = run_with_sizes(
            run_args(unhealthy, database_url.clone()),
            Sizes {
                read: 0,
                cycle: 0,
                hot: 0,
                ihe: 0,
            },
        )
        .await
        .expect_err("unhealthy server must fail")
        .to_string();
        assert!(error.contains("503 Service Unavailable"), "{error}");
        unhealthy_server.abort();

        let error = run_with_sizes(
            run_args("http://127.0.0.1:9".into(), database_url),
            Sizes {
                read: 0,
                cycle: 0,
                hot: 0,
                ihe: 0,
            },
        )
        .await
        .expect_err("missing server must fail")
        .to_string();
        assert!(error.contains("no server at"), "{error}");
    }

    #[tokio::test]
    async fn scenario_reporting_records_stress_knees_breaches_and_soak_windows() {
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let (base, server) = run_server("200 OK").await;
        let ctx = run_context(base);
        let strict = Budgets::parse(
            r#"{"min_samples":0,"default":{"p95_ms":-1,"p99_ms":-1,"max_error_rate":1}}"#,
        )
        .expect("strict budgets");
        let checked = Phase {
            name: "step-1".into(),
            workers: 1,
            duration: Duration::from_millis(15),
            rate: None,
            check: Check::Budgets,
        };

        let mut stress = report("stress");
        run_scenario(
            &ctx,
            Scenario::Read,
            std::slice::from_ref(&checked),
            &strict,
            &mut stress,
        )
        .await;
        assert_eq!(stress.knees.get("read").map(String::as_str), Some("step-1"));
        assert!(stress.breaches.is_empty(), "stress reports the knee once");

        let mut load = report("load");
        run_scenario(
            &ctx,
            Scenario::Read,
            std::slice::from_ref(&checked),
            &strict,
            &mut load,
        )
        .await;
        assert!(!load.breaches.is_empty());
        assert!(
            load.breaches
                .iter()
                .all(|breach| breach.phase.starts_with("read/"))
        );

        let unchecked = |name: &str| Phase {
            name: name.into(),
            workers: 1,
            duration: Duration::from_millis(5),
            rate: None,
            check: Check::None,
        };
        let mut soak = report("soak");
        run_scenario(
            &ctx,
            Scenario::Read,
            &[unchecked("minute-1"), unchecked("minute-2")],
            &strict,
            &mut soak,
        )
        .await;
        assert_eq!(soak.phases.len(), 2);
        server.abort();
    }

    #[test]
    fn a_soak_that_slows_down_is_drift() {
        let window = |name: &str, p95| {
            let mut ops = BTreeMap::new();
            ops.insert(
                "get_slot".to_string(),
                OpSummary {
                    count: 1000,
                    ok: 1000,
                    contended: 0,
                    errors: 0,
                    error_rate: 0.0,
                    per_second: 0.0,
                    mean_ms: 0.0,
                    p50_ms: 0.0,
                    p90_ms: 0.0,
                    p95_ms: p95,
                    p99_ms: p95,
                    max_ms: p95,
                    statuses: BTreeMap::new(),
                },
            );
            PhaseReport {
                scenario: "read",
                phase: name.into(),
                workers: 1,
                rate: None,
                seconds: 60.0,
                shed: 0,
                ops,
            }
        };
        assert!(soak_drift(&[&window("minute-1", 10.0), &window("minute-2", 24.0)]).is_empty());
        let d = soak_drift(&[&window("minute-1", 10.0), &window("minute-2", 26.0)]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].measure, "p95_drift_ms");

        assert!(soak_drift(&[]).is_empty());
        assert!(soak_drift(&[&window("minute-1", 10.0)]).is_empty());
        assert!(soak_drift(&[&window("warm-up", 10.0), &window("steady", 100.0)]).is_empty());
    }

    #[test]
    fn reports_print_in_closed_open_passed_and_failed_forms() {
        let mut ops = BTreeMap::new();
        ops.insert(
            "get_slot".to_string(),
            OpSummary {
                count: 2,
                ok: 1,
                contended: 1,
                errors: 0,
                error_rate: 0.0,
                per_second: 2.0,
                mean_ms: 1.0,
                p50_ms: 1.0,
                p90_ms: 2.0,
                p95_ms: 2.0,
                p99_ms: 2.0,
                max_ms: 2.0,
                statuses: BTreeMap::new(),
            },
        );
        let mut phase = PhaseReport {
            scenario: "read",
            phase: "steady".into(),
            workers: 2,
            rate: None,
            seconds: 1.0,
            shed: 0,
            ops,
        };
        print_phase(&phase);
        phase.rate = Some(10.0);
        phase.shed = 3;
        print_phase(&phase);

        let mut report = Report {
            run_id: "test".into(),
            url: "http://example.test".into(),
            profile: "smoke".into(),
            started_at: "2026-11-19T00:00:00Z".into(),
            phases: vec![phase],
            breaches: Vec::new(),
            knees: BTreeMap::new(),
            correctness: vec![Correctness {
                name: "double booking",
                passed: true,
                detail: "none".into(),
            }],
            passed: true,
        };
        print_verdict(&report);
        report.correctness[0].passed = false;
        report.breaches.push(Breach {
            phase: "steady".into(),
            op: "get_slot".into(),
            measure: "p95_ms",
            value: 3.0,
            limit: 2.0,
        });
        report.knees.insert("read", "step-16".into());
        report.passed = false;
        print_verdict(&report);
    }
}

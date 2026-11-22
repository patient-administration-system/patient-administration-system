//! Load shapes: the phases of each profile, and how one phase is driven.
//!
//! A phase is either **closed** (a fixed number of workers, each sending its next request when the last
//! one answers) or **open** (`--rate`: requests start at a fixed rate whatever the server does, with at
//! most `workers` in flight). A closed phase slows down with the server, so it hides queueing; an open one
//! does not, and a request it could not start because the limit was reached is counted as *shed*, so a
//! server that falls behind shows it.

use crate::scenarios::{Ctx, Rng, Scenario, once};
use crate::stats::Recorder;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// What a phase's results are held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Latency percentiles and the error rate, per operation, against the budgets.
    Budgets,
    /// The error rate only: latency is expected to rise (a spike, a stress step past the knee).
    ErrorsOnly,
    /// Not checked: a warm-up.
    None,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub name: String,
    pub workers: usize,
    pub duration: Duration,
    /// Requests per second for an open phase; `None` is a closed phase.
    pub rate: Option<f64>,
    pub check: Check,
}

impl Phase {
    fn new(name: impl Into<String>, workers: usize, secs: u64, check: Check) -> Self {
        Self {
            name: name.into(),
            workers,
            duration: Duration::from_secs(secs),
            rate: None,
            check,
        }
    }
}

/// The overrides a person can give on the command line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Overrides {
    pub workers: Option<usize>,
    pub duration: Option<Duration>,
    pub rate: Option<f64>,
}

/// The phases of a profile. `workers`, `duration` and `rate` change the main phase (the steady one, or
/// each soak window); they do not change a warm-up, a spike or the stress steps' sizes.
pub fn profile(name: &str, o: Overrides) -> Option<Vec<Phase>> {
    let main = |default_workers, default_secs: u64, name: &str| {
        let mut p = Phase::new(
            name,
            o.workers.unwrap_or(default_workers),
            default_secs,
            Check::Budgets,
        );
        if let Some(d) = o.duration {
            p.duration = d;
        }
        p.rate = o.rate;
        p
    };
    Some(match name {
        "smoke" => vec![main(4, 10, "steady")],
        "load" => vec![
            Phase::new("warm-up", 8, 10, Check::None),
            main(32, 60, "steady"),
        ],
        "stress" => [8, 16, 32, 64, 128, 256]
            .iter()
            .map(|&w| {
                let mut p = Phase::new(format!("step-{w}"), w, 20, Check::Budgets);
                if let Some(d) = o.duration {
                    p.duration = d;
                }
                p
            })
            .collect(),
        "spike" => vec![
            Phase::new("before", 4, 20, Check::Budgets),
            Phase::new("spike", 128, 20, Check::ErrorsOnly),
            Phase::new("after", 4, 20, Check::Budgets),
        ],
        "soak" => {
            // The soak is cut into one-minute windows so drift (a leak, a growing table, a slower query)
            // shows as a trend; the last window is compared with the first.
            let total = o.duration.unwrap_or(Duration::from_secs(30 * 60));
            let windows = (total.as_secs() / 60).max(1);
            let mut phases = vec![Phase::new("warm-up", 8, 10, Check::None)];
            for i in 1..=windows {
                let mut p = main(16, 60, &format!("minute-{i}"));
                p.duration = Duration::from_secs(60);
                phases.push(p);
            }
            phases
        }
        _ => return None,
    })
}

/// Drive one phase and return what it recorded and how long it took.
pub async fn run_phase(ctx: &Arc<Ctx>, scenario: Scenario, phase: &Phase) -> (Recorder, Duration) {
    let started = Instant::now();
    let rec = match phase.rate {
        None => closed(ctx, scenario, phase).await,
        Some(rate) => open(ctx, scenario, phase, rate).await,
    };
    // A worker that was mid-call at the deadline finishes it, so the wall time is measured at the end.
    (rec, started.elapsed())
}

async fn closed(ctx: &Arc<Ctx>, scenario: Scenario, phase: &Phase) -> Recorder {
    let deadline = Instant::now() + phase.duration;
    let mut set = JoinSet::new();
    for w in 0..phase.workers {
        let ctx = Arc::clone(ctx);
        set.spawn(async move {
            let mut rng = Rng::seeded(w);
            let mut rec = Recorder::default();
            while Instant::now() < deadline {
                once(scenario, &ctx, &mut rng, &mut rec).await;
            }
            rec
        });
    }
    collect(set).await
}

async fn open(ctx: &Arc<Ctx>, scenario: Scenario, phase: &Phase, rate: f64) -> Recorder {
    let deadline = Instant::now() + phase.duration;
    let limit = Arc::new(Semaphore::new(phase.workers.max(1)));
    // A rate that is not a positive finite number sends nothing: the empty recorder shows zero requests.
    let Ok(period) = Duration::try_from_secs_f64(1.0 / rate) else {
        return Recorder::default();
    };
    if period.is_zero() {
        return Recorder::default();
    }
    let mut tick = tokio::time::interval(period);
    // A late tick is still a request a client would have sent: send the ones that are due.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut set = JoinSet::new();
    let mut shed = 0;
    let mut n = 0usize;
    while Instant::now() < deadline {
        tick.tick().await;
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            shed += 1;
            continue;
        };
        n += 1;
        let ctx = Arc::clone(ctx);
        set.spawn(async move {
            let mut rng = Rng::seeded(n);
            let mut rec = Recorder::default();
            once(scenario, &ctx, &mut rng, &mut rec).await;
            drop(permit);
            rec
        });
    }
    let mut rec = collect(set).await;
    rec.shed = shed;
    rec
}

async fn collect(mut set: JoinSet<Recorder>) -> Recorder {
    let mut all = Recorder::default();
    while let Some(r) = set.join_next().await {
        if let Ok(r) = r {
            all.merge(r);
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Fixtures;
    use jiff::Timestamp;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    async fn test_server(delay: Duration) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let address = listener.local_addr().expect("test HTTP address");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let _read = socket.read(&mut request).await;
                    tokio::time::sleep(delay).await;
                    let _written = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await;
                });
            }
        });
        (format!("http://{address}"), server)
    }

    fn context(base: String) -> Arc<Ctx> {
        let now = Timestamp::now();
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

    #[test]
    fn every_profile_has_a_checked_phase_and_unknown_is_refused() {
        for name in ["smoke", "load", "stress", "spike", "soak"] {
            let phases = profile(name, Overrides::default()).unwrap_or_default();
            assert!(phases.iter().any(|p| p.check == Check::Budgets), "{name}");
        }
        assert!(profile("bogus", Overrides::default()).is_none());
    }

    #[test]
    fn overrides_change_the_main_phase_only() {
        let o = Overrides {
            workers: Some(3),
            duration: Some(Duration::from_secs(5)),
            rate: Some(50.0),
        };
        let load = profile("load", o).unwrap_or_default();
        assert_eq!(load[0].workers, 8, "the warm-up keeps its size");
        assert_eq!(
            (load[1].workers, load[1].duration, load[1].rate),
            (3, Duration::from_secs(5), Some(50.0))
        );
        let soak = profile(
            "soak",
            Overrides {
                duration: Some(Duration::from_secs(180)),
                ..Overrides::default()
            },
        )
        .unwrap_or_default();
        assert_eq!(soak.len(), 4, "a warm-up and three one-minute windows");

        let stress = profile(
            "stress",
            Overrides {
                duration: Some(Duration::from_secs(7)),
                ..Overrides::default()
            },
        )
        .unwrap_or_default();
        assert!(
            stress
                .iter()
                .all(|phase| phase.duration == Duration::from_secs(7))
        );
    }

    #[tokio::test]
    async fn closed_and_open_phases_drive_requests_and_report_shedding() {
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let (base, server) = test_server(Duration::from_millis(20)).await;
        let ctx = context(base);

        let closed_phase = Phase {
            name: "closed-test".into(),
            workers: 1,
            duration: Duration::from_millis(45),
            rate: None,
            check: Check::None,
        };
        let (closed_recorder, closed_wall) = run_phase(&ctx, Scenario::Read, &closed_phase).await;
        let closed_count: u64 = closed_recorder
            .summarise(closed_wall)
            .values()
            .fold(0_u64, |total, summary| total + summary.count);
        assert!(closed_count > 0);
        assert_eq!(closed_recorder.shed, 0);

        let open_phase = Phase {
            name: "open-test".into(),
            workers: 1,
            duration: Duration::from_millis(45),
            rate: Some(1_000.0),
            check: Check::None,
        };
        let (open_recorder, open_wall) = run_phase(&ctx, Scenario::Read, &open_phase).await;
        let open_count: u64 = open_recorder
            .summarise(open_wall)
            .values()
            .fold(0_u64, |total, summary| total + summary.count);
        assert!(open_count > 0);
        assert!(
            open_recorder.shed > 0,
            "the one in-flight permit must shed ticks"
        );

        let mut workers = JoinSet::new();
        workers.spawn(async { panic!("a failed worker must not discard successful workers") });
        workers.spawn(async {
            let mut recorder = Recorder::default();
            recorder.record(crate::stats::Sample {
                op: "survivor",
                status: Some(200),
                verdict: crate::stats::Verdict::Ok,
                elapsed: Duration::from_millis(1),
            });
            recorder
        });
        let collected = collect(workers).await;
        assert_eq!(
            collected
                .summarise(Duration::from_secs(1))
                .get("survivor")
                .map(|summary| summary.count),
            Some(1)
        );

        server.abort();
    }
}

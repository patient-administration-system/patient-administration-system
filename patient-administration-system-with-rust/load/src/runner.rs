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
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / rate.max(0.001)));
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
    }
}

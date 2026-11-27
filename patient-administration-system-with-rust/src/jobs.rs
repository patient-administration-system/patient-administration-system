//! Background jobs (QUAL-2): one runner for every timer loop in `main`.
//!
//! - **One at a time across replicas.** Each run takes the job's lease in
//!   `job_leases` (one statement; no connection is held while it works); a
//!   replica that does not get it skips that tick. A run releases its lease
//!   when it ends, and a crashed process's lease lapses after [`LEASE`].
//! - **A panic does not end the job.** Each run is its own task; a panic is
//!   logged and the next tick runs.
//! - **A failure is not a success.** The work returns a `Result`; an error is
//!   logged and counted as `Failed`, a missing connection as `Unavailable`,
//!   and neither moves the job's last success (PA-10, see `metrics`).
//! - **Clean shutdown.** `Jobs::stop` tells every loop to finish the run it
//!   is in and stop, and waits for them for a grace period.

use crate::db::sql::{Conn, Pool, Stmt};
use std::future::Future;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

/// What happened to one tick of a job. Only `Ran` is a success; `Skipped` is the normal answer on a
/// replica that did not get the lock; the other three are failures, counted apart so an operator can tell a
/// job that errs from one that cannot reach the database (audit PA-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// This process held the lock and the work succeeded.
    Ran,
    /// Another replica holds the lock: it runs the work this tick.
    Skipped,
    /// No connection, or the lock could not be asked for: nobody knows whether the work ran.
    Unavailable,
    /// The work ran and returned an error; the error was logged.
    Failed,
    /// The work panicked; the panic was logged.
    Panicked,
}

/// How long a run may hold its job before another replica may take it: a run that dies (a crash, a lost
/// network) blocks the job for no longer than this. A run that takes longer may overlap the next replica's,
/// which jobs already allow: each is safe to repeat.
pub const LEASE: Duration = Duration::from_secs(15 * 60);

/// Run `work` only if no other process is running the job `name`.
///
/// The job is held by a lease row in `job_leases`, taken and released each in one statement, so no connection
/// is held while the work runs and the work can use the whole pool, a pool of one included (audit PA-09). A
/// lease is taken when there is none or the last one has lapsed.
pub async fn run_exclusive<F, Fut>(db: &Pool, name: &str, work: F) -> Outcome
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = crate::Result<()>> + Send + 'static,
{
    let holder = uuid::Uuid::new_v4();
    let lease_secs = i64::try_from(LEASE.as_secs()).unwrap_or(i64::MAX);
    let taken = db
        .one(Stmt::new(
            "INSERT INTO job_leases (name, holder, held_until, taken_at) \
             VALUES ($1, $2, now() + make_interval(secs => $3), now()) \
             ON CONFLICT (name) DO UPDATE SET holder = EXCLUDED.holder, held_until = EXCLUDED.held_until, \
               taken_at = EXCLUDED.taken_at \
             WHERE job_leases.held_until < now() \
             RETURNING holder",
            [name.into(), holder.into(), lease_secs.into()],
        ))
        .await;
    match taken {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::debug!(target: "pas::jobs", job = name, "job is running elsewhere; skipped");
            return Outcome::Skipped;
        }
        Err(e) => {
            tracing::warn!(target: "pas::jobs", job = name, "job not run, no lease: {e}");
            return Outcome::Unavailable;
        }
    }
    let outcome = match tokio::spawn(work()).await {
        Ok(Ok(())) => Outcome::Ran,
        Ok(Err(e)) => {
            tracing::warn!(target: "pas::jobs", job = name, "job failed: {e}");
            Outcome::Failed
        }
        Err(e) => {
            tracing::error!(target: "pas::jobs", job = name, "job panicked or was cancelled: {e}");
            Outcome::Panicked
        }
    };
    // Give the job back at once; if this fails the lease lapses on its own.
    if let Err(e) = db
        .run(Stmt::new(
            "UPDATE job_leases SET held_until = now() WHERE name = $1 AND holder = $2",
            [name.into(), holder.into()],
        ))
        .await
    {
        tracing::warn!(target: "pas::jobs", job = name, "job lease not released (it lapses): {e}");
    }
    outcome
}

/// The set of running background jobs.
pub struct Jobs {
    db: Pool,
    stop: watch::Sender<bool>,
    handles: Vec<(&'static str, JoinHandle<()>)>,
}

impl Jobs {
    pub fn new(db: Pool) -> Self {
        Self {
            db,
            stop: watch::channel(false).0,
            handles: Vec::new(),
        }
    }

    /// Run `work` every `every`, the first time at once, until [`Jobs::stop`].
    pub fn spawn<F, Fut>(&mut self, name: &'static str, every: Duration, work: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        let db = self.db.clone();
        let mut stop = self.stop.subscribe();
        crate::metrics::job_registered(name, every);
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                // Stop wins when both are ready, so no run starts after `stop`.
                tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    _ = tick.tick() => {}
                }
                let outcome = run_exclusive(&db, name, &work).await;
                crate::metrics::job_finished(name, outcome);
            }
            tracing::info!(target: "pas::jobs", job = name, "job stopped");
        });
        self.handles.push((name, handle));
    }

    /// Ask every job to stop after its current run and wait up to `grace`.
    pub async fn stop(self, grace: Duration) {
        let _ = self.stop.send(true);
        for (name, handle) in self.handles {
            if tokio::time::timeout(grace, handle).await.is_err() {
                tracing::warn!(target: "pas::jobs", job = name, "job did not stop within {grace:?}");
            }
        }
    }
}

/// Resolves on Ctrl-C or, on Unix, SIGTERM (what a container runtime sends).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}

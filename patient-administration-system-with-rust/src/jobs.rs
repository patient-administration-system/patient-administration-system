//! Background jobs (QUAL-2): one runner for every timer loop in `main`.
//!
//! - **One at a time across replicas.** Each run takes a transaction-scoped
//!   Postgres advisory lock named for the job (`pg_try_advisory_xact_lock`);
//!   a replica that does not get it skips that tick. The lock is released by
//!   the transaction, so a crashed process cannot keep it.
//! - **A panic does not end the job.** Each run is its own task; a panic is
//!   logged and the next tick runs.
//! - **Clean shutdown.** `Jobs::stop` tells every loop to finish the run it
//!   is in and stop, and waits for them for a grace period.

use std::future::Future;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// What happened to one tick of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// This process held the lock and ran the work.
    Ran,
    /// Another replica holds the lock (or the lock could not be asked for).
    Skipped,
    /// The work panicked; the panic was logged.
    Panicked,
}

/// Run `work` only if no other process is running the job `name`.
pub async fn run_exclusive<F, Fut>(db: &DatabaseConnection, name: &str, work: F) -> Outcome
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let txn = match db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "pas::jobs", job = name, "job skipped, no connection: {e}");
            return Outcome::Skipped;
        }
    };
    let got = txn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0)) AS got",
            [format!("pas.job.{name}").into()],
        ))
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<bool>("", "got").ok())
        .unwrap_or(false);
    if !got {
        tracing::debug!(target: "pas::jobs", job = name, "job is running elsewhere; skipped");
        let _ = txn.rollback().await;
        return Outcome::Skipped;
    }
    let outcome = match tokio::spawn(work()).await {
        Ok(()) => Outcome::Ran,
        Err(e) => {
            tracing::error!(target: "pas::jobs", job = name, "job panicked or was cancelled: {e}");
            Outcome::Panicked
        }
    };
    let _ = txn.commit().await;
    outcome
}

/// The set of running background jobs.
pub struct Jobs {
    db: DatabaseConnection,
    stop: watch::Sender<bool>,
    handles: Vec<(&'static str, JoinHandle<()>)>,
}

impl Jobs {
    pub fn new(db: DatabaseConnection) -> Self {
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
        Fut: Future<Output = ()> + Send + 'static,
    {
        let db = self.db.clone();
        let mut stop = self.stop.subscribe();
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                // Stop wins when both are ready, so no run starts after `stop`.
                tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    _ = tick.tick() => {}
                }
                run_exclusive(&db, name, &work).await;
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

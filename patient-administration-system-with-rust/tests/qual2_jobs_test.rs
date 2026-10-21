//! QUAL-2: background jobs run one at a time across replicas, survive a panic and stop on request.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use patient_administration_system::jobs::{Jobs, Outcome, run_exclusive};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use uuid::Uuid;

async fn db() -> sea_orm::DatabaseConnection {
    common::build_state(&common::database_url().unwrap())
        .await
        .db
}

/// A job name no other test or process uses.
fn name() -> String {
    format!("qual2-{}", Uuid::new_v4())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_replicas_cannot_run_the_same_job_at_once() {
    let db = db().await;
    let job = name();
    let runs = Arc::new(AtomicUsize::new(0));
    let slow = |runs: Arc<AtomicUsize>| {
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    };
    let (a, b) = tokio::join!(
        run_exclusive(&db, &job, slow(runs.clone())),
        run_exclusive(&db, &job, slow(runs.clone())),
    );
    let mut outcomes = [a, b];
    outcomes.sort_by_key(|o| format!("{o:?}"));
    assert_eq!(outcomes, [Outcome::Ran, Outcome::Skipped]);
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the work ran exactly once");

    // The lock goes with the transaction: the next tick runs.
    let again = run_exclusive(&db, &job, slow(runs.clone())).await;
    assert_eq!(again, Outcome::Ran);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn different_jobs_do_not_block_each_other() {
    let db = db().await;
    let (one, two) = (name(), name());
    let (a, b) = tokio::join!(
        run_exclusive(&db, &one, || async {
            tokio::time::sleep(Duration::from_millis(200)).await
        }),
        run_exclusive(&db, &two, || async {
            tokio::time::sleep(Duration::from_millis(200)).await
        }),
    );
    assert_eq!((a, b), (Outcome::Ran, Outcome::Ran));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_panic_is_reported_and_the_next_tick_still_runs() {
    let db = db().await;
    let job = name();
    let first = run_exclusive(&db, &job, || async { panic!("boom") }).await;
    assert_eq!(first, Outcome::Panicked);
    let next = run_exclusive(&db, &job, || async {}).await;
    assert_eq!(next, Outcome::Ran, "the lock was released after the panic");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_panicking_job_keeps_ticking_and_stop_finishes_the_current_run() {
    let db = db().await;
    let ticks = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut jobs = Jobs::new(db);
    let (t, f) = (ticks.clone(), finished.clone());
    jobs.spawn("qual2-loop", Duration::from_millis(50), move || {
        let (t, f) = (t.clone(), f.clone());
        async move {
            let n = t.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                panic!("first run fails");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            f.fetch_add(1, Ordering::SeqCst);
        }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        ticks.load(Ordering::SeqCst) >= 2,
        "the loop outlived the panic"
    );
    let started = ticks.load(Ordering::SeqCst);
    jobs.stop(Duration::from_secs(5)).await;
    // Every run that started finished (the first one panicked, so one fewer finished).
    assert_eq!(
        finished.load(Ordering::SeqCst) + 1,
        ticks.load(Ordering::SeqCst)
    );
    assert!(
        ticks.load(Ordering::SeqCst) <= started + 1,
        "no new runs after stop"
    );
}

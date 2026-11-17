//! HUB-6, NFR-34: `GET /api/health` sees a stalled worker. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Its own test binary, because which jobs run is process-wide (`metrics::job_registered`), and the other
//! health tests run without a dispatcher. (A dispatcher that never finishes a tick becomes `stalled` after
//! three intervals and at least 30 s; `metrics` unit tests cover that without waiting.)
mod common;
use axum::{body::Body, http::Request};
use patient_administration_system::api::rest::router;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::jobs::Outcome;
use patient_administration_system::metrics::{job_finished, job_registered};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

async fn health(state: &patient_administration_system::api::rest::AppState) -> serde_json::Value {
    let resp = router(state.clone())
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["data"].clone()
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn health_reports_the_dispatcher_and_degrades_on_an_old_backlog() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    // Before any dispatcher runs in this process: off, and the backlog's age is not held against it.
    let h = health(&state).await;
    assert_eq!(h["outbox"]["dispatcher"], "off");
    assert_eq!(h["mllp_listener"], "off");

    // Started, no tick finished yet: unknown, never ok.
    job_registered("outbox", Duration::from_secs(2));
    assert_eq!(health(&state).await["outbox"]["dispatcher"], "unknown");

    job_finished("outbox", Outcome::Ran);
    // An event undelivered for a day, past PAS_OUTBOX_STALE_SECONDS (1800 by default).
    let id = Uuid::new_v4();
    state
        .db
        .run(Stmt::new(
            "INSERT INTO outbox_events (id, event_type, payload, published, at, retry_count) \
             VALUES ($1, 'Hub6Test', '{}'::jsonb, false, now() - interval '1 day', 0)",
            [id.into()],
        ))
        .await
        .unwrap();
    let h = health(&state).await;
    assert_eq!(h["outbox"]["dispatcher"], "ok");
    assert!(
        h["outbox"]["oldest_undelivered_seconds"].as_i64().unwrap() >= 86_000,
        "{h}"
    );
    assert_eq!(
        h["status"], "degraded",
        "a backlog nobody delivers is not healthy: {h}"
    );
    state
        .db
        .run(Stmt::new(
            "DELETE FROM outbox_events WHERE id = $1",
            [id.into()],
        ))
        .await
        .unwrap();
}

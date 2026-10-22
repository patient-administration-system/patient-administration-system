//! QUAL-16: a request id on every request, and the numbers an operator needs. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::jobs::{Outcome, run_exclusive};
use patient_administration_system::observability::with_request_telemetry;
use tower::ServiceExt;
use uuid::Uuid;

async fn app() -> (axum::Router, sea_orm::DatabaseConnection) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let db = s.db.clone();
    (
        with_request_telemetry(patient_administration_system::api::rest::router(s)),
        db,
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_response_carries_a_request_id_made_or_echoed() {
    let (app, _) = app().await;
    // None sent: one is made, and it is a UUID.
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let made = r.headers()["x-request-id"].to_str().unwrap().to_string();
    assert!(Uuid::parse_str(&made).is_ok(), "{made}");
    // Two requests, two ids.
    let r2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(made, r2.headers()["x-request-id"].to_str().unwrap());
    // One sent by the caller (a proxy): it is kept and echoed, also on an error.
    let r3 = app
        .oneshot(
            Request::builder()
                .uri("/api/no-such-route")
                .header("x-request-id", "proxy-abc-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::NOT_FOUND);
    assert_eq!(r3.headers()["x-request-id"], "proxy-abc-123");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_metrics_show_requests_jobs_the_outbox_and_the_call_list() {
    let (app, db) = app().await;
    let job = format!("qual16-{}", Uuid::new_v4());
    assert_eq!(run_exclusive(&db, &job, || async {}).await, Outcome::Ran);
    patient_administration_system::metrics::job_finished(&job, Outcome::Ran);
    // A 404 so the 4xx counter has something.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/no-such-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let r = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    let text =
        String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    for needle in [
        "pas_http_requests_total{class=\"4xx\"}",
        "pas_outbox_pending ",
        "pas_outbox_oldest_pending_seconds ",
        "pas_outbox_dead_letters ",
        "pas_staff_call_tasks_open ",
    ] {
        assert!(text.contains(needle), "missing {needle}:\n{text}");
    }
    assert!(
        text.contains(&format!(
            "pas_job_runs_total{{job=\"{job}\",outcome=\"ran\"}} 1"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "pas_job_last_success_timestamp_seconds{{job=\"{job}\"}}"
        )),
        "{text}"
    );
    // Numbers only: nothing that looks like an id (a UUID) other than the job name this test made up.
    let without_job = text.replace(&job, "JOB");
    assert!(
        !without_job
            .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
            .any(|w| Uuid::parse_str(w).is_ok()),
        "an id in the metrics:\n{without_job}"
    );
}

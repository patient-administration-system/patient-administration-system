//! Smoke test: the REST router answers `GET /api/health` with `200 OK`.
//!
//! Skipped silently when `DATABASE_URL` is not set, so plain `cargo test`
//! works on a developer machine without Postgres.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::upstream::HttpPersonClient;
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn health_endpoint_returns_ok() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping health_endpoint_returns_ok");
            return;
        }
    };
    let state = common::build_state(&url).await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(resp.status(), StatusCode::OK);
}

/// NFR-28: a configured service that does not answer makes health `degraded`, still with HTTP 200, and the
/// response does not say where the service is.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn health_reports_an_unreachable_upstream() {
    let Some(url) = common::database_url() else {
        eprintln!("DATABASE_URL not set; skipping health_reports_an_unreachable_upstream");
        return;
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let closed = format!("http://{}", listener.local_addr().expect("address"));
    drop(listener);
    let person =
        HttpPersonClient::new(closed.clone(), Duration::from_secs(30), None).expect("client");
    let state = common::build_state(&url)
        .await
        .with_person_client(Arc::new(person));

    let (status, text) = health(state).await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(body["data"]["status"], "degraded");
    assert_eq!(body["data"]["database"], "ok");
    assert_eq!(body["data"]["person_service"], "unreachable");
    assert_eq!(body["data"]["worker_service"], "not_configured");
    assert!(
        !text.contains(&closed),
        "the address stays out of an open route"
    );
}

/// NFR-28: a service with no URL shows as such and is not a fault.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn health_is_ok_with_no_upstream_configured() {
    let Some(url) = common::database_url() else {
        eprintln!("DATABASE_URL not set; skipping health_is_ok_with_no_upstream_configured");
        return;
    };
    let (status, text) = health(common::build_state(&url).await).await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(body["data"]["status"], "ok");
    assert_eq!(body["data"]["person_service"], "not_configured");
    assert_eq!(body["data"]["worker_service"], "not_configured");
}

async fn health(state: AppState) -> (StatusCode, String) {
    let resp = router(state)
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

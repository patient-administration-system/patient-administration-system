//! Integration tests for the v0.44.0 Phase 3a `/api/practitioners/*`
//! proxy over `crate::upstream::WorkerClient`. Mirror of
//! `upstream_person_proxy_test.rs`.

mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::{StubWorkerClient, Worker};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn worker_with_id(id: Uuid, family: &str) -> Worker {
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "family": family, "given": ["Sam"] }),
    );
    data.insert("active".into(), serde_json::json!(true));
    Worker { id, data }
}

#[tokio::test]
async fn test_get_practitioner_returns_seeded_worker() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubWorkerClient::new());
    let id = Uuid::new_v4();
    stub.insert(worker_with_id(id, "Brown"));

    let state = common::build_state(&url).await.with_worker_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/practitioners/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["id"], serde_json::json!(id));
    assert_eq!(env["data"]["name"]["family"], serde_json::json!("Brown"));
}

#[tokio::test]
async fn test_create_practitioner_returns_new_id() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubWorkerClient::new());
    let state = common::build_state(&url)
        .await
        .with_worker_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "name": { "family": "Lee", "given": ["Kim"] },
        "active": true,
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/practitioners")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert!(env["data"]["id"].is_string());
    assert_eq!(stub.len(), 1);
}

#[tokio::test]
async fn test_practitioner_proxy_returns_503_when_unset() {
    let Some(url) = common::database_url() else {
        return;
    };
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/practitioners/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

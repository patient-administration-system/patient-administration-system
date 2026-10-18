//! Integration tests for the v0.44.0 Phase 3a `/api/patients/*` proxy
//! over `crate::upstream::PersonClient`. Uses the in-memory
//! `StubPersonClient` so the tests work offline.
//!
//! Gated on `DATABASE_URL` only because `AppState::new` requires a real
//! `DatabaseConnection` for its non-proxy services. The proxy code path
//! itself does not query the DB.

mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::{Person, StubPersonClient};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn person_with_id(id: Uuid, family: &str) -> Person {
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "family": family, "given": ["Jane"] }),
    );
    data.insert("active".into(), serde_json::json!(true));
    Person { id, data }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_get_patient_returns_seeded_person() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let id = Uuid::new_v4();
    stub.insert(person_with_id(id, "Doe"));

    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["success"], Value::Bool(true));
    assert_eq!(env["data"]["id"], serde_json::json!(id));
    assert_eq!(env["data"]["name"]["family"], serde_json::json!("Doe"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_get_unknown_patient_returns_404() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let env = body_json(resp).await;
    assert_eq!(env["success"], Value::Bool(false));
    assert_eq!(env["error"]["code"], serde_json::json!("NOT_FOUND"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_create_patient_returns_new_id() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let state = common::build_state(&url)
        .await
        .with_person_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "name": { "family": "Smith", "given": ["Pat"] },
        "active": true,
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/patients")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert!(
        env["data"]["id"].is_string(),
        "id should be present, got {env:?}"
    );
    assert_eq!(stub.len(), 1, "stub should have one person");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_delete_patient_removes_from_stub() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let id = Uuid::new_v4();
    stub.insert(person_with_id(id, "Doe"));
    let state = common::build_state(&url)
        .await
        .with_person_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/patients/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(stub.is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_proxy_returns_503_when_upstream_client_unset() {
    let Some(url) = common::database_url() else {
        return;
    };
    // No with_person_client call → AppState.person_client is None.
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let env = body_json(resp).await;
    assert_eq!(
        env["error"]["code"],
        serde_json::json!("UPSTREAM_UNAVAILABLE")
    );
}

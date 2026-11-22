//! The `/api/practitioners/*` proxy over `WorkerClient`, with the in-memory `StubWorkerClient`: every route
//! reaches the client, a body that is not an object is refused, and with no worker service every route says so
//! with a 503. Needs PostgreSQL only because `AppState::new` does: `cargo test -- --ignored`.
mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::StubWorkerClient;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_practitioner_is_created_read_changed_searched_exported_and_deleted_through_the_client() {
    let stub = Arc::new(StubWorkerClient::new());
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_worker_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let (st, v) = call(
        &app,
        "POST",
        "/api/practitioners",
        Some(serde_json::json!({ "name": { "family": "Okafor" }, "active": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(stub.len(), 1);
    let (st, _) = call(
        &app,
        "POST",
        "/api/practitioners",
        Some(serde_json::json!("nope")),
    )
    .await;
    assert!(st.is_client_error(), "a body that is not an object");

    let (st, v) = call(&app, "GET", &format!("/api/practitioners/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["name"]["family"], "Okafor");
    let (st, _) = call(
        &app,
        "GET",
        &format!("/api/practitioners/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = call(
        &app,
        "PUT",
        &format!("/api/practitioners/{id}"),
        Some(serde_json::json!({ "name": { "family": "Eze" } })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["name"]["family"], "Eze");
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/practitioners/{id}"),
        Some(serde_json::json!(7)),
    )
    .await;
    assert!(st.is_client_error());
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/practitioners/{}", Uuid::new_v4()),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = call(&app, "GET", "/api/practitioners?q=Eze&limit=5", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["data"].is_array());
    let (st, _) = call(&app, "GET", "/api/practitioners", None).await;
    assert_eq!(st, StatusCode::OK);

    for tail in ["masked", "export", "audit?limit=3"] {
        let (st, v) = call(
            &app,
            "GET",
            &format!("/api/practitioners/{id}/{tail}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{tail}: {v}");
    }

    let (st, _) = call(&app, "DELETE", &format!("/api/practitioners/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(stub.is_empty());
    let (st, _) = call(&app, "DELETE", &format!("/api/practitioners/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_practitioner_route_says_so_when_there_is_no_worker_service() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state);
    let id = Uuid::new_v4();
    for (method, uri, body) in [
        (
            "POST",
            "/api/practitioners".to_string(),
            Some(serde_json::json!({})),
        ),
        ("GET", format!("/api/practitioners/{id}"), None),
        (
            "PUT",
            format!("/api/practitioners/{id}"),
            Some(serde_json::json!({})),
        ),
        ("DELETE", format!("/api/practitioners/{id}"), None),
        ("GET", "/api/practitioners?q=x".to_string(), None),
        ("GET", format!("/api/practitioners/{id}/masked"), None),
        ("GET", format!("/api/practitioners/{id}/export"), None),
        ("GET", format!("/api/practitioners/{id}/audit"), None),
    ] {
        let (st, _) = call(&app, method, &uri, body).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
    }
}

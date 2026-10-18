//! Integration tests for the v0.44.0 Phase 3a inbound merge webhooks:
//! `POST /api/upstream/person-merged` and
//! `POST /api/upstream/worker-merged`.
//!
//! These tests do touch the DB — they exercise `upstream_inbox` dedupe
//! and audit_log writes — so they gate on `DATABASE_URL`. They run
//! against a freshly-migrated DB.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret).unwrap();
    mac.update(body);
    format!(
        "sha256={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

#[tokio::test]
async fn test_person_merged_webhook_without_secret_returns_503() {
    let Some(url) = common::database_url() else {
        return;
    };
    // SAFETY: tests are serial via cargo's per-binary process model; only
    // one test in this binary touches PERSON_SERVICE_WEBHOOK_SECRET
    // at a time.
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_person_merged_webhook_wrong_signature_returns_401() {
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "right");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let bad_sig = sign(b"wrong", &body_bytes);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", bad_sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let env = body_json(resp).await;
    assert_eq!(env["error"]["code"], serde_json::json!("UNSIGNED_WEBHOOK"));
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
async fn test_person_merged_webhook_happy_path_applied_then_dedupe() {
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "shared");
    }
    let state = common::build_state(&url).await;

    // Two fresh PAS-owned ids; nothing else in the DB references them.
    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
        "event_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"shared", &body_bytes);

    // First call — applied.
    let resp = patient_administration_system::api::rest::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", &sig)
                .body(Body::from(body_bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(true));

    // Second call with the same body — duplicate (idempotent).
    let resp = patient_administration_system::api::rest::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", &sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(false));
    assert_eq!(env["data"]["reason"], serde_json::json!("duplicate"));

    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
async fn test_person_merged_self_merge_returns_400() {
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "shared");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);
    let same = Uuid::new_v4();
    let body = serde_json::json!({ "source_id": same, "target_id": same });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"shared", &body_bytes);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
async fn test_worker_merged_webhook_happy_path() {
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("WORKER_SERVICE_WEBHOOK_SECRET", "wsecret");
    }
    let state = common::build_state(&url).await;
    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
        "event_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"wsecret", &body_bytes);
    let resp = patient_administration_system::api::rest::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/worker-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(true));
    unsafe {
        std::env::remove_var("WORKER_SERVICE_WEBHOOK_SECRET");
    }
}

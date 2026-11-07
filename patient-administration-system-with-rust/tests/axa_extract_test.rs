//! A bad body, path or query is refused with a problem that says what was wrong and quotes nothing the caller
//! sent (`axa::extract`). Axum's own extractors answered plain text, which can quote the request. Needs
//! PostgreSQL for the router's state: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, String, serde_json::Value) {
    let r = app.clone().oneshot(request).await.unwrap();
    let status = r.status();
    let kind = r
        .headers()
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (
        status,
        kind,
        serde_json::from_slice(&bytes).unwrap_or_default(),
    )
}

fn post(uri: &str, kind: Option<&str>, body: &str) -> Request<Body> {
    let mut r = Request::post(uri);
    if let Some(kind) = kind {
        r = r.header(CONTENT_TYPE, kind);
    }
    r.body(Body::from(body.to_string())).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bad_request_is_a_problem_that_quotes_nothing() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let json = Some("application/json");
    let cases = [
        (
            "path",
            get("/api/appointments/secret-id/history"),
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            "query",
            get("/api/referrals?limit=secret"),
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            "syntax",
            post("/api/referrals", json, "{secret"),
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            "shape",
            post("/api/referrals", json, r#"{"secret":1}"#),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unprocessable_entity",
        ),
        (
            "media type",
            post("/api/referrals", None, "secret"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ),
    ];
    for (name, request, status, code) in cases {
        let (got, kind, body) = call(&app, request).await;
        assert_eq!(got, status, "{name}: {body}");
        assert_eq!(kind, "application/problem+json", "{name}: {body}");
        assert_eq!(body["code"], code, "{name}: {body}");
        assert!(body["detail"].is_string(), "{name}: {body}");
        assert!(!body.to_string().contains("secret"), "{name}: {body}");
    }
}

/// An optional body (`OptionalJson`) may be left out, and a malformed one is still refused.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_optional_body_may_be_left_out_but_not_malformed() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let uri = format!("/api/referrals/{}/close", uuid::Uuid::new_v4());
    let (status, _, body) = call(&app, post(&uri, Some("application/json"), "{secret")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request", "{body}");
    assert!(!body.to_string().contains("secret"), "{body}");
    // With no body the request gets past the extractor, to the handler's own refusal.
    let (status, _, body) = call(&app, post(&uri, None, "")).await;
    assert_ne!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_ne!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
}

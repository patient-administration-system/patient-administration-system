//! The limit per client and the lockout after repeated `401`s (`api::rest::rate_limit`, axa-ratelimit).
//!
//! Needs no database: each test wraps a tiny router with `with_rate_limits`, and gives each request the
//! `ConnectInfo` that `axum::serve` would, so that clients are told apart by address.

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use patient_administration_system::api::rest::{RateLimits, with_rate_limits};
use tower::ServiceExt;

/// `/test` answers `200`; `/secret` answers `401` unless the request says `authorization: good`.
fn build_app(limits: RateLimits) -> Router {
    let routes = Router::new()
        .route("/test", get(|| async { "ok" }))
        .route("/api/health", get(|| async { "ok" }))
        .route(
            "/secret",
            get(|headers: axum::http::HeaderMap| async move {
                if headers.get("authorization").is_some_and(|v| v == "good") {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            }),
        );
    with_rate_limits(routes, limits)
}

async fn send(app: &Router, uri: &str, from: Option<&str>, auth: Option<&str>) -> Response {
    let mut request = Request::builder().uri(uri);
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    let mut request = request.body(Body::empty()).unwrap();
    if let Some(from) = from {
        let peer: SocketAddr = format!("{from}:40000").parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    app.clone().oneshot(request).await.unwrap()
}

async fn status(app: &Router, uri: &str, from: Option<&str>) -> StatusCode {
    send(app, uri, from, None).await.status()
}

#[tokio::test]
async fn a_client_over_its_limit_gets_a_429_problem() {
    let app = build_app(RateLimits::new(2, 0));
    let a = Some("203.0.113.1");
    assert_eq!(status(&app, "/test", a).await, StatusCode::OK);
    let ok = send(&app, "/test", a, None).await;
    assert_eq!(ok.headers()["ratelimit-remaining"], "0");

    let resp = send(&app, "/test", a, None).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().get("retry-after").is_some());
    assert_eq!(resp.headers()["content-type"], "application/problem+json");
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["code"], "too_many_requests", "{v}");
}

#[tokio::test]
async fn each_address_has_its_own_limit_and_health_is_never_counted() {
    let app = build_app(RateLimits::new(1, 0));
    let a = Some("203.0.113.1");
    let b = Some("198.51.100.7");
    assert_eq!(status(&app, "/test", a).await, StatusCode::OK);
    assert_eq!(
        status(&app, "/test", a).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        status(&app, "/test", b).await,
        StatusCode::OK,
        "another client is not held up"
    );
    for _ in 0..5 {
        assert_eq!(status(&app, "/api/health", a).await, StatusCode::OK);
    }
}

/// With no `ConnectInfo`, as when a test drives the router, requests share one limit rather than none.
#[tokio::test]
async fn requests_with_no_address_share_one_limit() {
    let app = build_app(RateLimits::new(1, 0));
    assert_eq!(status(&app, "/test", None).await, StatusCode::OK);
    assert_eq!(
        status(&app, "/test", None).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn repeated_401s_lock_a_client_out_and_successes_do_not_count() {
    let app = build_app(RateLimits::new(0, 2));
    let a = Some("203.0.113.1");
    for _ in 0..5 {
        let good = send(&app, "/secret", a, Some("good")).await;
        assert_eq!(
            good.status(),
            StatusCode::OK,
            "signing in correctly is never slowed"
        );
    }
    assert_eq!(status(&app, "/secret", a).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(&app, "/secret", a).await, StatusCode::UNAUTHORIZED);
    let locked = send(&app, "/secret", a, Some("good")).await;
    assert_eq!(
        locked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "locked out, even with the right token"
    );
    assert_eq!(
        status(&app, "/test", a).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    let other = send(&app, "/secret", Some("198.51.100.7"), Some("good")).await;
    assert_eq!(
        other.status(),
        StatusCode::OK,
        "another client is not locked out"
    );
}

#[tokio::test]
async fn zero_turns_each_layer_off() {
    let app = build_app(RateLimits::new(0, 0));
    let a = Some("203.0.113.1");
    for _ in 0..50 {
        assert_eq!(status(&app, "/test", a).await, StatusCode::OK);
        assert_eq!(status(&app, "/secret", a).await, StatusCode::UNAUTHORIZED);
    }
}

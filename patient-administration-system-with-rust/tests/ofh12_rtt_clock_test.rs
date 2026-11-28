//! OFH-12: an RTT clock moves only by the allowed steps (active to paused or stopped, paused to active or
//! stopped, stopped to nothing), and pause, resume and stop check the version read from `GET /api/rtt/{id}`.
//! A refused move is `409`, writes no clock event and leaves the status as it was. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const OLD: &str = "\"2000-01-01T00:00:00.000000Z\"";

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    if_match: Option<&str>,
    body: Value,
) -> (StatusCode, Option<String>, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = if_match {
        b = b.header("if-match", t);
    }
    let r = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let st = r.status();
    let tag = r
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let v = serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
        .unwrap_or_default();
    (st, tag, v)
}

async fn world() -> (AppState, Router) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    (s, app)
}

async fn started(app: &Router) -> String {
    let (st, _, v) = call(
        app,
        "POST",
        "/api/rtt/start",
        None,
        json!({"person_id": Uuid::new_v4(), "target_service": format!("ofh12-{}", Uuid::new_v4())}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["data"]["id"].as_str().unwrap().to_string()
}

/// The pathway's status and its version, read as a client would.
async fn read(app: &Router, id: &str) -> (String, String) {
    let (st, tag, v) = call(app, "GET", &format!("/api/rtt/{id}"), None, Value::Null).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["id"], id);
    let tag = tag.expect("GET /api/rtt/{id} sends an ETag");
    (v["data"]["status"].as_str().unwrap().to_string(), tag)
}

async fn events(s: &AppState, id: &str) -> i64 {
    s.db.one(Stmt::new(
        "SELECT count(*)::bigint AS n FROM rtt_clock_events WHERE pathway_id = $1",
        [Uuid::parse_str(id).unwrap().into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .col("n")
    .unwrap()
}

/// One move with the version just read; returns the status code.
async fn step(app: &Router, id: &str, action: &str) -> StatusCode {
    let (_, tag) = read(app, id).await;
    let (st, _, v) = call(
        app,
        "POST",
        &format!("/api/rtt/{id}/{action}"),
        Some(&tag),
        json!({"reason": "ofh12"}),
    )
    .await;
    if st == StatusCode::CONFLICT {
        assert_eq!(v["code"], "conflict", "{v}");
    }
    st
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_clock_moves_only_by_the_allowed_steps() {
    let (s, app) = world().await;
    let id = started(&app).await;
    assert_eq!(read(&app, &id).await.0, "active");

    // A running clock cannot be resumed.
    assert_eq!(step(&app, &id, "resume").await, StatusCode::CONFLICT);
    assert_eq!(step(&app, &id, "pause").await, StatusCode::OK);
    // A paused clock cannot be paused again.
    assert_eq!(step(&app, &id, "pause").await, StatusCode::CONFLICT);
    assert_eq!(read(&app, &id).await.0, "paused");
    assert_eq!(step(&app, &id, "resume").await, StatusCode::OK);
    assert_eq!(step(&app, &id, "pause").await, StatusCode::OK);
    // A paused clock can stop.
    assert_eq!(step(&app, &id, "stop").await, StatusCode::OK);
    // A stopped clock stays stopped.
    for action in ["resume", "pause", "stop"] {
        assert_eq!(
            step(&app, &id, action).await,
            StatusCode::CONFLICT,
            "{action}"
        );
    }
    assert_eq!(read(&app, &id).await.0, "stopped");
    // started, paused, resumed, paused, stopped: the refused moves wrote nothing.
    assert_eq!(events(&s, &id).await, 5);

    // And an active clock can stop directly.
    let other = started(&app).await;
    assert_eq!(step(&app, &other, "stop").await, StatusCode::OK);
    assert_eq!(step(&app, &other, "resume").await, StatusCode::CONFLICT);
    assert_eq!(events(&s, &other).await, 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn pause_resume_and_stop_check_the_version() {
    let (s, app) = world().await;
    let id = started(&app).await;
    for action in ["pause", "resume", "stop"] {
        let uri = format!("/api/rtt/{id}/{action}");
        let body = json!({"reason": "ofh12"});
        let (st, _, v) = call(&app, "POST", &uri, None, body.clone()).await;
        assert_eq!(st, StatusCode::PRECONDITION_REQUIRED, "{action}: {v}");
        let (st, _, v) = call(&app, "POST", &uri, Some(OLD), body).await;
        assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{action}: {v}");
    }
    assert_eq!(events(&s, &id).await, 1, "only the start");

    // The version moves with each change, so the one read before a pause is stale after it.
    let (_, before) = read(&app, &id).await;
    assert_eq!(step(&app, &id, "pause").await, StatusCode::OK);
    let (st, _, _) = call(
        &app,
        "POST",
        &format!("/api/rtt/{id}/resume"),
        Some(&before),
        json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    // `*` is any version (an inbound feed that does not track one).
    let (st, _, v) = call(
        &app,
        "POST",
        &format!("/api/rtt/{id}/resume"),
        Some("*"),
        json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(read(&app, &id).await.0, "active");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_unknown_pathway_is_not_found() {
    let (_s, app) = world().await;
    let id = Uuid::new_v4();
    let (st, _, _) = call(&app, "GET", &format!("/api/rtt/{id}"), None, Value::Null).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = call(
        &app,
        "POST",
        &format!("/api/rtt/{id}/pause"),
        Some("*"),
        json!({"reason": "ofh12"}),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

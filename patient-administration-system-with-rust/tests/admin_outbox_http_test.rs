//! The administrator's outbox endpoints over HTTP: list what is unpublished and dead-lettered, replay or skip
//! a dead letter (each audited), and read the metrics. Needs PostgreSQL: `cargo test -- --ignored`.
mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, String) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, String::from_utf8_lossy(&bytes).into_owned())
}

async fn dead_letter(db: &patient_administration_system::db::sql::Pool, tag: &str) -> (Uuid, Uuid) {
    let (id, original) = (Uuid::new_v4(), Uuid::new_v4());
    db.run(Stmt::new(
        "INSERT INTO outbox_dead_letters (id, original_id, event_type, payload, created_at, retry_count, last_error) \
         VALUES ($1, $2, $3, '{}'::jsonb, now(), 10, 'gave up')",
        [id.into(), original.into(), tag.to_string().into()],
    ))
    .await
    .unwrap();
    (id, original)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn dead_letters_are_listed_replayed_and_skipped_and_each_is_audited() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let db = s.db.clone();
    let app = patient_administration_system::api::rest::router(s);
    let tag = format!("admin-http-{}", Uuid::new_v4().simple());
    let (to_replay, _) = dead_letter(&db, &tag).await;
    let (to_skip, original) = dead_letter(&db, &tag).await;

    let (st, body) = call(&app, "GET", "/api/admin/outbox/dead-letters?limit=200").await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let (st, _) = call(&app, "GET", "/api/admin/outbox/dead-letters?limit=lots").await;
    assert!(st.is_client_error(), "a limit that is not a number");
    let (st, _) = call(&app, "GET", "/api/admin/outbox/unpublished?limit=5").await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(&app, "GET", "/api/admin/outbox/unpublished?limit=0").await;
    assert!(st.is_client_error(), "a limit outside 1 to the maximum");

    // Replay puts the event back in the outbox and removes the dead letter; a second replay finds nothing.
    let (st, body) = call(
        &app,
        "POST",
        &format!("/api/admin/outbox/dead-letters/{to_replay}/replay"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body.contains(&to_replay.to_string()));
    let (st, _) = call(
        &app,
        "POST",
        &format!("/api/admin/outbox/dead-letters/{to_replay}/replay"),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Skip gives up on the other, naming what it was.
    let (st, body) = call(
        &app,
        "POST",
        &format!("/api/admin/outbox/dead-letters/{to_skip}/skip"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body.contains(&original.to_string()) && body.contains(&tag));
    let (st, _) = call(
        &app,
        "POST",
        &format!("/api/admin/outbox/dead-letters/{to_skip}/skip"),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    for (id, action) in [(to_replay, "replay"), (to_skip, "skip")] {
        let n = db
            .one(Stmt::new(
                "SELECT count(*) AS n FROM audit_log WHERE entity_type = 'outbox_dead_letter' AND entity_id = $1 AND action = $2",
                [id.into(), action.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col::<i64>("n")
            .unwrap();
        assert_eq!(n, 1, "{action} is audited once");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_metrics_are_prometheus_text() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let r = app
        .oneshot(
            Request::get("/api/admin/metrics")
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
            .starts_with("text/plain; version=0.0.4")
    );
    let text =
        String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(text.contains("# TYPE") || text.contains("# HELP"), "{text}");
}

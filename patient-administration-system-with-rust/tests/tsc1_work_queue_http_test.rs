//! TSC-1 over HTTP: the work-queue handlers' own branches (who is acting, the filters, the error answers).
//! Needs PostgreSQL: `cargo test -- --ignored`. The service rules are in `tsc1_work_queue_test.rs`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::work_queue::WorkQueueService;
use tower::ServiceExt;
use uuid::Uuid;

async fn app() -> (Router, patient_administration_system::db::sql::Pool) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let db = s.db.clone();
    (patient_administration_system::api::rest::router(s), db)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    who: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(w) = who {
        b = b.header("x-user-id", w);
    }
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// A call task due soon, synced into the queue; returns the work item's id.
async fn open_item(db: &patient_administration_system::db::sql::Pool) -> Uuid {
    let task = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,$4,$5)",
        [
            task.into(),
            Uuid::new_v4().into(),
            Uuid::new_v4().into(),
            "silent".into(),
            (Timestamp::now() + SignedDuration::from_hours(1)).into(),
        ],
    ))
    .await
    .unwrap();
    WorkQueueService::new(db.clone())
        .sync(Timestamp::now())
        .await
        .unwrap();
    db.one(Stmt::new(
        "SELECT id FROM work_items WHERE source_id = $1",
        [task.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .col("id")
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn claim_release_and_assign_over_http_act_for_the_signed_in_person() {
    let (app, db) = app().await;
    let id = open_item(&db).await;
    let sam = format!("sam-{}", Uuid::new_v4());
    let kim = format!("kim-{}", Uuid::new_v4());
    let item = format!("/api/work-items/{id}");

    let (st, _) = call(&app, "GET", &item, None, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &app,
        "GET",
        &format!("/api/work-items/{}", Uuid::new_v4()),
        None,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // A trusted service acting for a person: the item is the person's.
    let acting = format!("svc:ui for {sam}");
    let (st, v) = call(&app, "POST", &format!("{item}/claim"), Some(&acting), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(&app, "POST", &format!("{item}/claim"), Some(&kim), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _) = call(&app, "POST", &format!("{item}/release"), Some(&kim), None).await;
    assert_ne!(st, StatusCode::OK, "only the owner releases");

    // The filters: mine, nobody's, a named person, one queue, overdue.
    let (_, mine) = call(
        &app,
        "GET",
        "/api/work-items?owner=me&limit=200",
        Some(&sam),
        None,
    )
    .await;
    assert!(mine.to_string().contains(&id.to_string()));
    let (_, theirs) = call(
        &app,
        "GET",
        &format!("/api/work-items?owner={sam}"),
        None,
        None,
    )
    .await;
    assert!(theirs.to_string().contains(&id.to_string()));
    let (_, none) = call(&app, "GET", "/api/work-items?owner=none", None, None).await;
    assert!(!none.to_string().contains(&id.to_string()));
    let (st, _) = call(
        &app,
        "GET",
        "/api/work-items?queue=booking&overdue=true",
        None,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(&app, "GET", "/api/work-items?owner=me", None, None).await;
    assert!(st.is_client_error(), "owner=me needs a signed-in person");

    let (st, _) = call(&app, "POST", &format!("{item}/release"), Some(&sam), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(
        &app,
        "POST",
        &format!("{item}/assign"),
        Some(&sam),
        Some(serde_json::json!({ "to": kim })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        &app,
        "POST",
        &format!("{item}/assign"),
        Some(&sam),
        Some(serde_json::json!({ "to": "  " })),
    )
    .await;
    assert!(st.is_client_error());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reminders_and_take_next_over_http() {
    let (app, db) = app().await;
    let _ = open_item(&db).await;
    let sam = format!("sam-{}", Uuid::new_v4());

    let (st, _) = call(&app, "GET", "/api/work-items/reminders", None, None).await;
    assert!(st.is_client_error(), "reminders need a signed-in person");
    let (st, _) = call(&app, "GET", "/api/work-items/reminders", Some(&sam), None).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _) = call(&app, "POST", "/api/work-queues/operations/next", None, None).await;
    assert!(st.is_client_error() || st == StatusCode::OK);
    let (st, v) = call(
        &app,
        "POST",
        "/api/work-queues/not-a-queue/next",
        Some(&sam),
        None,
    )
    .await;
    assert!(st.is_client_error(), "{st} {v}");
}

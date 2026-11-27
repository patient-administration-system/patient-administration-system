//! TSC-6 for FHIR: `GET /fhir/Appointment?_lastUpdated=…` gives the appointments changed in a window, oldest
//! change first, a page at a time; a deleted one is `entered-in-error`. The REST list says which are deleted.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use tower::ServiceExt;
use uuid::Uuid;

/// A day of its own in the past, so no other test's rows are in the window.
fn window() -> (Timestamp, Timestamp) {
    let day = i64::try_from(Uuid::new_v4().as_u128() % 10_000).unwrap();
    let since = Timestamp::from_second(946_684_800).unwrap() + SignedDuration::from_hours(day * 24);
    (since, since + SignedDuration::from_hours(24))
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let b = to_bytes(r.into_body(), 4 << 20).await.unwrap();
    (status, serde_json::from_slice(&b).unwrap_or_default())
}

async fn appointment(db: &Pool, changed: Timestamp) -> Uuid {
    let start = Timestamp::now() + SignedDuration::from_hours(48);
    let mut a = Appointment::new(Uuid::new_v4(), start, start + SignedDuration::from_mins(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(db, &a).await.unwrap();
    db.run(Stmt::new(
        "UPDATE appointments SET updated_at = $2 WHERE id = $1",
        [a.id.into(), changed.into()],
    ))
    .await
    .unwrap();
    a.id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_fhir_poll_walks_the_window_oldest_change_first_and_sees_a_deletion() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let db = state.db.clone();
    let (since, before) = window();
    let h = |n: i64| since + SignedDuration::from_hours(n);
    let _earlier = appointment(&db, since - SignedDuration::from_secs(1)).await;
    let late = appointment(&db, h(5)).await;
    let early = appointment(&db, h(1)).await;
    let deleted = appointment(&db, h(3)).await;
    let _later = appointment(&db, before).await;
    db.run(Stmt::new(
        "UPDATE appointments SET deleted_at = $2 WHERE id = $1",
        [deleted.into(), h(3).into()],
    ))
    .await
    .unwrap();

    let fhir = fhir_router(state.clone());
    let mut next = Some(format!(
        "/fhir/Appointment?_lastUpdated=ge{since}&_lastUpdated=lt{before}&_count=1"
    ));
    let mut seen = Vec::new();
    while let Some(uri) = next.take() {
        let (st, b) = get(&fhir, &uri.replace('+', "%2B")).await;
        assert_eq!(st, StatusCode::OK, "{b}");
        assert_eq!(b["type"], "searchset");
        for e in b["entry"].as_array().unwrap() {
            let r = &e["resource"];
            seen.push((
                r["id"].as_str().unwrap().to_string(),
                r["status"].as_str().unwrap().to_string(),
            ));
            assert!(r["meta"]["lastUpdated"].is_string(), "{r}");
        }
        next = b["link"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["relation"] == "next")
            .map(|l| {
                let url = l["url"].as_str().unwrap();
                url[url.find("/fhir/").unwrap()..].to_string()
            });
        assert!(seen.len() <= 3, "the walk ends");
    }
    assert_eq!(
        seen,
        vec![
            (early.to_string(), "booked".to_string()),
            (deleted.to_string(), "entered-in-error".to_string()),
            (late.to_string(), "booked".to_string()),
        ]
    );

    // gt and le move each bound by a microsecond: the row changed exactly at `since` is out, at `h(5)` in.
    let (_, b) = get(
        &fhir,
        &format!(
            "/fhir/Appointment?_lastUpdated=gt{}&_lastUpdated=le{}",
            h(1),
            h(5)
        )
        .replace('+', "%2B"),
    )
    .await;
    let ids: Vec<&str> = b["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["resource"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![deleted.to_string(), late.to_string()]);

    for bad in [
        "/fhir/Appointment",
        "/fhir/Appointment?_lastUpdated=lt2026-01-01",
        "/fhir/Appointment?_lastUpdated=ge2026-01-02&_lastUpdated=lt2026-01-01",
        "/fhir/Appointment?_lastUpdated=ge2026-01-01&_count=0",
        "/fhir/Appointment?_lastUpdated=ge2026-01-01&patient=Patient/x",
    ] {
        let (st, b) = get(&fhir, bad).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}: {b}");
        assert_eq!(b["resourceType"], "OperationOutcome");
    }

    // The REST list of what changed says which appointment is deleted.
    let rest = patient_administration_system::api::rest::router(state);
    let (st, b) = get(
        &rest,
        &format!("/api/appointments?updated_since={since}&updated_before={before}&limit=10")
            .replace('+', "%2B"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let rows = b["data"].as_array().unwrap();
    let find = |id: Uuid| rows.iter().find(|r| r["id"] == id.to_string()).unwrap();
    assert!(find(deleted)["deleted_at"].is_string());
    assert!(find(early)["deleted_at"].is_null());
}

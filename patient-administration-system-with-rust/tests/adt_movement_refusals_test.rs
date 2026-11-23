//! ADT movements that must be refused, and leave the beds as they were: a transfer to the bed already held,
//! to a bed someone else holds, a transfer or its cancellation for an admission that has no bed or no transfer.
//! Needs PostgreSQL: `cargo test -- --ignored`.
mod common;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::models::Address;
use patient_administration_system::models::facility::{Bed, Facility, Room, Ward};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("if-match", "*")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let v = serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
        .unwrap_or_default();
    (status, v)
}

fn text<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

/// A ward with `n` free beds, as bed ids.
async fn beds(app: &Router, n: usize) -> Vec<Uuid> {
    let unique = Uuid::new_v4().simple().to_string();
    let facility = Facility::new("F".into(), format!("F{unique}"), Address::default());
    call(app, "POST", "/api/facilities", &text(&facility)).await;
    let ward = Ward::new(facility.id, "W".into(), format!("W{unique}"), 8);
    call(app, "POST", "/api/wards", &text(&ward)).await;
    let room = Room::new(ward.id, "R".into(), format!("R{unique}"));
    call(app, "POST", "/api/rooms", &text(&room)).await;
    let mut out = Vec::new();
    for i in 0..n {
        let bed = Bed::new(room.id, format!("B{i}"), format!("B{i}{unique}"));
        call(app, "POST", "/api/beds", &text(&bed)).await;
        out.push(bed.id);
    }
    out
}

async fn admit(app: &Router, bed: Uuid) -> String {
    let body = serde_json::json!({ "person_id": Uuid::new_v4(), "bed_id": bed }).to_string();
    let (status, v) = call(app, "POST", "/api/admissions", &body).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v["data"]["admission"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_transfer_to_the_same_bed_or_to_a_held_bed_is_refused() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let b = beds(&app, 3).await;
    let a = admit(&app, b[0]).await;
    let other = admit(&app, b[1]).await;
    let to = |bed: Uuid| serde_json::json!({ "new_bed_id": bed }).to_string();

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/transfer"),
        &to(b[0]),
    )
    .await;
    assert!(
        status.is_client_error() && status != StatusCode::CONFLICT,
        "{status} {v}"
    );
    assert!(v.to_string().contains("same bed"), "{v}");

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/transfer"),
        &to(b[1]),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v.to_string().contains("not available"), "{v}");

    // Nothing moved: the patient is still in their own bed and a transfer to the free one works.
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/transfer"),
        &to(b[2]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let _ = other;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_transfer_or_its_cancellation_with_nothing_to_move_is_not_found() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let b = beds(&app, 2).await;
    let a = admit(&app, b[0]).await;

    // No transfer has been made, so there is none to cancel.
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/cancel-transfer"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert!(v.to_string().contains("no transfer to cancel"), "{v}");

    // Once discharged the admission holds no bed, so neither a transfer nor a cancellation has anything to act on.
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/discharge"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let body = serde_json::json!({ "new_bed_id": b[1] }).to_string();
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/transfer"),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert!(v.to_string().contains("no active bed assignment"), "{v}");
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/admissions/{a}/cancel-transfer"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
}

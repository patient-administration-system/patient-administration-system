//! OFH-1 (NFR-27): optimistic concurrency. A read returns the record's version as `ETag`; a write sends it
//! back as `If-Match`; a missing one is 428, a stale one is 412, and neither changes anything.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::waitlist::Priority;
use tower::ServiceExt;
use uuid::Uuid;

async fn world() -> (patient_administration_system::api::rest::AppState, Router) {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state.clone());
    (state, app)
}

/// `(status, ETag header, body)`.
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    if_match: Option<&str>,
    body: &str,
) -> (StatusCode, Option<String>, serde_json::Value) {
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

async fn booked(state: &patient_administration_system::api::rest::AppState) -> Appointment {
    let start = Utc::now() + Duration::days(9);
    let mut a = Appointment::new(Uuid::new_v4(), start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(&state.db, &a).await.unwrap()
}

async fn status_of(
    state: &patient_administration_system::api::rest::AppState,
    id: Uuid,
) -> AppointmentStatus {
    AppointmentRepository::find_by_id(&state.db, id)
        .await
        .unwrap()
        .unwrap()
        .status
}

/// Two writers read the same appointment. The first cancels. The second, holding the old version, gets 412
/// and changes nothing; a write with no `If-Match` is 428 and changes nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_second_writer_of_one_appointment_gets_412_and_a_missing_version_gets_428() {
    let (s, app) = world().await;
    let a = booked(&s).await;
    let uri = format!("/api/appointments/{}", a.id);

    let (st, tag_a, _) = call(&app, "GET", &uri, None, "").await;
    assert_eq!(st, StatusCode::OK);
    let tag_a = tag_a.expect("a read of one appointment carries an ETag");
    let (_, tag_b, _) = call(&app, "GET", &uri, None, "").await;
    assert_eq!(
        Some(&tag_a),
        tag_b.as_ref(),
        "two readers see the same version"
    );

    let (st, _, v) = call(&app, "POST", &format!("{uri}/cancel"), None, "{}").await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED, "{v}");
    assert_eq!(v["code"], "PRECONDITION_REQUIRED");
    assert_eq!(status_of(&s, a.id).await, AppointmentStatus::Booked);

    let (st, new_tag, _) = call(&app, "POST", &format!("{uri}/cancel"), Some(&tag_a), "{}").await;
    assert_eq!(st, StatusCode::OK);
    assert_ne!(
        new_tag.as_deref(),
        Some(tag_a.as_str()),
        "the version moved"
    );

    let (st, _, v) = call(&app, "POST", &format!("{uri}/cancel"), Some(&tag_a), "{}").await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{v}");
    assert_eq!(v["code"], "PRECONDITION_FAILED");
    let cancels = s
        .db
        .one(Stmt::new("SELECT count(*) AS n FROM audit_log WHERE entity_type = 'appointment' AND entity_id = $1 AND action = 'cancel'",
            [a.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cancels.col::<i64>("n").unwrap(),
        1,
        "the refused write left no audit row"
    );
}

/// Two cancels with the same version sent at the same instant: the row lock makes one win (200) and the other
/// lose with 412, never both pass the check.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_simultaneous_writes_with_one_version_cannot_both_succeed() {
    let (s, app) = world().await;
    for _ in 0..5 {
        let a = booked(&s).await;
        let uri = format!("/api/appointments/{}", a.id);
        let (_, tag, _) = call(&app, "GET", &uri, None, "").await;
        let tag = tag.unwrap();
        let cancel = format!("{uri}/cancel");
        let ((s1, _, _), (s2, _, _)) = tokio::join!(
            call(&app, "POST", &cancel, Some(&tag), "{}"),
            call(&app, "POST", &cancel, Some(&tag), "{}")
        );
        let mut got = [s1.as_u16(), s2.as_u16()];
        got.sort_unstable();
        assert_eq!(got, [200, 412], "one wins, the other is told it is stale");
    }
}

/// The waiting list: an entry carries its version, a write needs it, a stale one is refused.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_waiting_list_entry_has_a_version_and_a_stale_write_is_refused() {
    let (s, app) = world().await;
    let entry = s
        .waitlist
        .add(
            None,
            Uuid::new_v4(),
            "cardiology".into(),
            Priority::Routine,
            &UserContext::system(),
        )
        .await
        .unwrap();
    let uri = format!("/api/waitlist/{}", entry.id);
    let (st, tag, _) = call(&app, "GET", &uri, None, "").await;
    assert_eq!(st, StatusCode::OK);
    let tag = tag.expect("an entry carries an ETag");

    // A clinical-priority change moves the version.
    let prio = format!("{uri}/clinical-priority");
    let body = r#"{"clinical_priority":"P2","review_by":"2099-01-01","set_by":"Dr A"}"#;
    let (st, _, _) = call(&app, "PUT", &prio, None, body).await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED);
    let (st, moved, v) = call(&app, "PUT", &prio, Some(&tag), body).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_ne!(moved.as_deref(), Some(tag.as_str()));

    // The old version is now stale for a remove, and for another priority change.
    let (st, _, _) = call(&app, "DELETE", &uri, Some(&tag), "").await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _, _) = call(&app, "PUT", &prio, Some(&tag), body).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _, _) = call(&app, "DELETE", &uri, None, "").await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED);
    let (st, _, _) = call(&app, "DELETE", &uri, moved.as_deref(), "").await;
    assert_eq!(st, StatusCode::OK);
}

/// An admission: its version moves with every movement, a movement needs it, and the second of two writers
/// who read the same version is refused.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_admissions_version_moves_with_every_movement() {
    use patient_administration_system::models::Address;
    use patient_administration_system::models::facility::{Bed, Facility, Room, Ward};
    let (_s, app) = world().await;
    let unique = Uuid::new_v4().simple().to_string();
    let facility = Facility::new("F".into(), format!("F{unique}"), Address::default());
    call(&app, "POST", "/api/facilities", Some("*"), &json(&facility)).await;
    let ward = Ward::new(facility.id, "W".into(), format!("W{unique}"), 4);
    call(&app, "POST", "/api/wards", Some("*"), &json(&ward)).await;
    let room = Room::new(ward.id, "R".into(), format!("R{unique}"));
    call(&app, "POST", "/api/rooms", Some("*"), &json(&room)).await;
    let mut beds = Vec::new();
    for n in 0..3 {
        let bed = Bed::new(room.id, format!("B{n}"), format!("B{n}{unique}"));
        call(&app, "POST", "/api/beds", Some("*"), &json(&bed)).await;
        beds.push(bed.id);
    }
    let person = Uuid::new_v4();
    let (st, _, v) = call(
        &app,
        "POST",
        "/api/admissions",
        None,
        &serde_json::json!({"person_id": person, "bed_id": beds[0]}).to_string(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["data"]["admission"]["id"].as_str().unwrap().to_string();
    let uri = format!("/api/admissions/{id}");

    let (st, tag, _) = call(&app, "GET", &uri, None, "").await;
    assert_eq!(st, StatusCode::OK);
    let tag = tag.expect("an admission carries an ETag");
    let to = |bed: Uuid| serde_json::json!({ "new_bed_id": bed }).to_string();

    let (st, _, _) = call(&app, "POST", &format!("{uri}/transfer"), None, &to(beds[1])).await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED);
    let (st, _, v) = call(
        &app,
        "POST",
        &format!("{uri}/transfer"),
        Some(&tag),
        &to(beds[1]),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // A transfer does not touch the admission row's own columns, yet the version moved.
    let (_, moved, _) = call(&app, "GET", &uri, None, "").await;
    assert_ne!(
        moved.as_deref(),
        Some(tag.as_str()),
        "a transfer moves the version"
    );

    // The second writer, still holding the old version, is refused and nothing moves.
    let (st, _, _) = call(
        &app,
        "POST",
        &format!("{uri}/transfer"),
        Some(&tag),
        &to(beds[2]),
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _, _) = call(&app, "POST", &format!("{uri}/discharge"), Some(&tag), "").await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (_, still, _) = call(&app, "GET", &uri, None, "").await;
    assert_eq!(still, moved, "refused writes do not move the version");

    let (st, _, v) = call(
        &app,
        "POST",
        &format!("{uri}/discharge"),
        moved.as_deref(),
        "",
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

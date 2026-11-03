//! OFH-10 (NFR-27): the rest of the write routes check a version. For each family: no `If-Match` is 428, an
//! old version is 412, the version just read is 200, and the refused ones changed nothing. And two admissions
//! into one bed at once (H-07): one wins, the other is refused. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::bed::BedRepository;
use patient_administration_system::db::repositories::encounter::EncounterRepository;
use patient_administration_system::models::Address;
use patient_administration_system::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancelledBy,
};
use patient_administration_system::models::encounter::{
    Encounter, EncounterClass, EncounterStatus,
};
use patient_administration_system::models::facility::{Bed, BedStatus, Facility, Room, Ward};
use patient_administration_system::scheduling::CancelDetails;
use sea_orm::ConnectionTrait;
use tower::ServiceExt;
use uuid::Uuid;

const OLD: &str = "\"2000-01-01T00:00:00.000000Z\"";

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    if_match: Option<&str>,
    body: serde_json::Value,
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

async fn world() -> (AppState, Router) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    (s, app)
}

async fn booked(s: &AppState) -> Appointment {
    let start = Utc::now() + Duration::days(9);
    let mut a = Appointment::new(Uuid::new_v4(), start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(&s.db, &a).await.unwrap()
}

async fn status_of(s: &AppState, id: Uuid) -> AppointmentStatus {
    AppointmentRepository::find_by_id(&s.db, id)
        .await
        .unwrap()
        .unwrap()
        .status
}

/// The three answers for one route: no version is 428, an old one is 412, the one just read is 200.
/// `check` runs after the two refusals and says the record is unchanged.
async fn three_answers<F, Fut>(
    app: &Router,
    read: &str,
    method: &str,
    write: &str,
    body: serde_json::Value,
    unchanged: F,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let (st, tag, _) = call(app, "GET", read, None, serde_json::json!({})).await;
    assert_eq!(st, StatusCode::OK, "{read}");
    let tag = tag.unwrap_or_else(|| panic!("{read} carries an ETag"));
    let (st, _, v) = call(app, method, write, None, body.clone()).await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED, "{write}: {v}");
    let (st, _, v) = call(app, method, write, Some(OLD), body.clone()).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{write}: {v}");
    assert!(unchanged().await, "{write}: a refused write changed it");
    let (st, _, v) = call(app, method, write, Some(&tag), body).await;
    assert_eq!(st, StatusCode::OK, "{write}: {v}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn check_in_complete_no_show_location_and_barrier_check_the_appointment_version() {
    let (s, app) = world().await;

    let a = booked(&s).await;
    let uri = format!("/api/appointments/{}", a.id);
    three_answers(
        &app,
        &uri,
        "POST",
        &format!("{uri}/check-in"),
        serde_json::json!({}),
        || async { status_of(&s, a.id).await == AppointmentStatus::Booked },
    )
    .await;
    assert_eq!(status_of(&s, a.id).await, AppointmentStatus::Arrived);

    // The same appointment, now arrived: location, then complete.
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/location"),
        serde_json::json!({"room": "Room 4"}),
        || async {
            AppointmentRepository::find_by_id(&s.db, a.id)
                .await
                .unwrap()
                .unwrap()
                .current_room
                .is_none()
        },
    )
    .await;
    three_answers(
        &app,
        &uri,
        "POST",
        &format!("{uri}/complete"),
        serde_json::json!({}),
        || async { status_of(&s, a.id).await == AppointmentStatus::Arrived },
    )
    .await;
    assert_eq!(status_of(&s, a.id).await, AppointmentStatus::Fulfilled);

    let m = booked(&s).await;
    let uri = format!("/api/appointments/{}", m.id);
    three_answers(
        &app,
        &uri,
        "POST",
        &format!("{uri}/no-show"),
        serde_json::json!({}),
        || async { status_of(&s, m.id).await == AppointmentStatus::Booked },
    )
    .await;
    assert_eq!(status_of(&s, m.id).await, AppointmentStatus::NoShow);

    // A barrier is recorded after a miss.
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/barrier"),
        serde_json::json!({"barrier_code": "work"}),
        || async {
            AppointmentRepository::find_by_id(&s.db, m.id)
                .await
                .unwrap()
                .unwrap()
                .barrier_code
                .is_none()
        },
    )
    .await;
    let c = booked(&s).await;
    s.scheduling
        .cancel_with(
            c.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: None,
                legacy_reason: None,
            },
            &UserContext::system(),
        )
        .await
        .unwrap();
    let uri = format!("/api/appointments/{}", c.id);
    let (_, tag, _) = call(&app, "GET", &uri, None, serde_json::json!({})).await;
    let (st, _, _) = call(
        &app,
        "PUT",
        &format!("{uri}/barrier"),
        tag.as_deref(),
        serde_json::json!({"barrier_code": BarrierCode::Caring}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}

/// Two check-ins carrying the version both callers read, sent together: one wins, the other is refused with
/// 412 (not left to fail on the state change), and the appointment is checked in once.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_simultaneous_check_ins_with_one_version_cannot_both_pass() {
    let (s, app) = world().await;
    for _ in 0..4 {
        let a = booked(&s).await;
        let uri = format!("/api/appointments/{}", a.id);
        let (_, tag, _) = call(&app, "GET", &uri, None, serde_json::json!({})).await;
        let tag = tag.unwrap();
        let path = format!("{uri}/check-in");
        let ((s1, _, _), (s2, _, _)) = tokio::join!(
            call(&app, "POST", &path, Some(&tag), serde_json::json!({})),
            call(&app, "POST", &path, Some(&tag), serde_json::json!({}))
        );
        let mut got = [s1, s2];
        got.sort();
        assert_eq!(
            got,
            [StatusCode::OK, StatusCode::PRECONDITION_FAILED],
            "exactly one wins"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn encounter_status_and_cancel_check_the_encounter_version() {
    let (s, app) = world().await;
    let person = Uuid::new_v4();
    let e = EncounterRepository::create(&s.db, &Encounter::new(person, EncounterClass::Outpatient))
        .await
        .unwrap();
    let uri = format!("/api/encounters/{}", e.id);
    let now = || async {
        EncounterRepository::find_by_id(&s.db, e.id)
            .await
            .unwrap()
            .unwrap()
            .status
    };
    let before = now().await;
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/status"),
        serde_json::json!({"status": "arrived"}),
        || async { now().await == before },
    )
    .await;
    assert_eq!(now().await, EncounterStatus::Arrived);

    let e2 =
        EncounterRepository::create(&s.db, &Encounter::new(person, EncounterClass::Outpatient))
            .await
            .unwrap();
    let uri = format!("/api/encounters/{}", e2.id);
    three_answers(
        &app,
        &uri,
        "POST",
        &format!("{uri}/cancel"),
        serde_json::json!({}),
        || async {
            EncounterRepository::find_by_id(&s.db, e2.id)
                .await
                .unwrap()
                .unwrap()
                .status
                != EncounterStatus::Cancelled
        },
    )
    .await;
}

/// A facility, ward, room and `n` available beds.
async fn beds(app: &Router, n: usize) -> Vec<Uuid> {
    let u = Uuid::new_v4().simple().to_string();
    let f = Facility::new("F".into(), format!("F{u}"), Address::default());
    call(
        app,
        "POST",
        "/api/facilities",
        Some("*"),
        serde_json::json!(f),
    )
    .await;
    let w = Ward::new(f.id, "W".into(), format!("W{u}"), 4);
    call(app, "POST", "/api/wards", Some("*"), serde_json::json!(w)).await;
    let r = Room::new(w.id, "R".into(), format!("R{u}"));
    call(app, "POST", "/api/rooms", Some("*"), serde_json::json!(r)).await;
    let mut out = Vec::new();
    for i in 0..n {
        let b = Bed::new(r.id, format!("B{i}"), format!("B{i}{u}"));
        call(app, "POST", "/api/beds", Some("*"), serde_json::json!(b)).await;
        out.push(b.id);
    }
    out
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bed_status_change_checks_the_bed_version() {
    let (s, app) = world().await;
    let bed = beds(&app, 1).await[0];
    let uri = format!("/api/beds/{bed}");
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/status"),
        serde_json::json!({"status": "out_of_service"}),
        || async {
            BedRepository::find_by_id(&s.db, bed)
                .await
                .unwrap()
                .unwrap()
                .status
                == BedStatus::Available
        },
    )
    .await;
}

/// H-07: two different people admitted to one bed at the same instant. The bed's row lock lets one in; the
/// other is refused with 409 and nothing of theirs is written. Same for two pre-admissions to one bed.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_admissions_into_one_bed_at_once_admit_one() {
    let (s, app) = world().await;
    for _ in 0..4 {
        let bed = beds(&app, 1).await[0];
        let (p1, p2) = (Uuid::new_v4(), Uuid::new_v4());
        let ((s1, _, _), (s2, _, _)) = tokio::join!(
            call(
                &app,
                "POST",
                "/api/admissions",
                Some("*"),
                serde_json::json!({"person_id": p1, "bed_id": bed})
            ),
            call(
                &app,
                "POST",
                "/api/admissions",
                Some("*"),
                serde_json::json!({"person_id": p2, "bed_id": bed})
            )
        );
        let mut got = [s1, s2];
        got.sort();
        assert_eq!(got, [StatusCode::OK, StatusCode::CONFLICT], "one wins");
        let active = s
            .db
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Postgres,
                "SELECT count(*) AS n FROM bed_assignments WHERE bed_id = $1 AND released_at IS NULL",
                [bed.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.try_get::<i64>("", "n").unwrap(), 1, "one occupant");
    }
    for _ in 0..4 {
        let bed = beds(&app, 1).await[0];
        let ((s1, _, b1), (s2, _, b2)) = tokio::join!(
            call(
                &app,
                "POST",
                "/api/admissions/pre-admit",
                Some("*"),
                serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": bed})
            ),
            call(
                &app,
                "POST",
                "/api/admissions/pre-admit",
                Some("*"),
                serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": bed})
            )
        );
        let mut got = [s1, s2];
        got.sort();
        assert_eq!(
            got,
            [StatusCode::OK, StatusCode::CONFLICT],
            "one holds it: {b1} {b2}"
        );
    }
}

/// OFH-11: a slot's capacity and ring-fence need the slot's version (read from `GET /api/slots/{id}`).
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn slot_capacity_and_ring_fence_check_the_slot_version() {
    let (s, app) = world().await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 12).await;
    let uri = format!("/api/slots/{}", slot.id);
    let stored = || async {
        s.db.query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Postgres,
            "SELECT capacity, ring_fenced FROM slots WHERE id = $1",
            [slot.id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
    };
    let before = stored().await.try_get::<i32>("", "capacity").unwrap();
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/capacity"),
        serde_json::json!({"capacity": before + 3, "set_by": "Clinic lead"}),
        || async { stored().await.try_get::<i32>("", "capacity").unwrap() == before },
    )
    .await;
    three_answers(
        &app,
        &uri,
        "PUT",
        &format!("{uri}/ring-fence"),
        serde_json::json!({"ring_fenced": true, "set_by": "Clinic lead"}),
        || async { !stored().await.try_get::<bool>("", "ring_fenced").unwrap() },
    )
    .await;
}

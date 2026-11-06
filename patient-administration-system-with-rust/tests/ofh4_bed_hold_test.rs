//! OFH-4: a pre-admission records a reservation for the bed; a hold with a time lapses and gives the bed back;
//! a cancel closes it; every change is an occupancy event. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::Address;
use patient_administration_system::models::facility::{Bed, BedStatus, Facility, Room, Ward};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
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
    let st = r.status();
    let v = serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
        .unwrap_or_default();
    (st, v)
}

/// A facility, ward and room, and `n` available beds.
async fn beds(app: &Router, n: usize) -> Vec<Uuid> {
    let u = Uuid::new_v4().simple().to_string();
    let f = Facility::new("F".into(), format!("F{u}"), Address::default());
    call(app, "POST", "/api/facilities", serde_json::json!(f)).await;
    let w = Ward::new(f.id, "W".into(), format!("W{u}"), 4);
    call(app, "POST", "/api/wards", serde_json::json!(w)).await;
    let r = Room::new(w.id, "R".into(), format!("R{u}"));
    call(app, "POST", "/api/rooms", serde_json::json!(r)).await;
    let mut out = Vec::new();
    for i in 0..n {
        let b = Bed::new(r.id, format!("B{i}"), format!("B{i}{u}"));
        call(app, "POST", "/api/beds", serde_json::json!(b)).await;
        out.push(b.id);
    }
    out
}

async fn bed_status(s: &AppState, id: Uuid) -> BedStatus {
    patient_administration_system::db::repositories::bed::BedRepository::find_by_id(&s.db, id)
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn events(app: &Router, bed: Uuid) -> Vec<String> {
    let (_, v) = call(
        app,
        "GET",
        &format!("/api/beds/{bed}/occupancy-events"),
        serde_json::json!({}),
    )
    .await;
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .rev() // newest first on the wire; oldest first here
        .map(|e| e["event"].as_str().unwrap().to_string())
        .collect()
}

async fn world() -> (AppState, Router) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    (s, app)
}

/// A pre-admission with a hold time records the reservation; once the time has passed the sweep gives the bed
/// back, cancels the planned encounter and records it. A pre-admission with none never lapses.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_lapsed_hold_gives_the_bed_back_and_one_with_no_time_never_does() {
    let (s, app) = world().await;
    let b = beds(&app, 2).await;
    let (held, open) = (b[0], b[1]);
    let person = Uuid::new_v4();

    let (st, v) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": person, "bed_id": held, "hold_minutes": 30}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let enc = v["data"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": open}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bed_status(&s, held).await, BedStatus::Reserved);

    // Before the time: nothing lapses.
    s.adt.release_expired_holds(Utc::now()).await.unwrap();
    assert_eq!(bed_status(&s, held).await, BedStatus::Reserved);

    // After it: this bed lapses; the one with no hold time does not, however late.
    let later = Utc::now() + Duration::days(400);
    // Other tests on this database may have holds of their own, so count this bed's, not the sweep's.
    assert!(s.adt.release_expired_holds(later).await.unwrap() >= 1);
    assert_eq!(bed_status(&s, held).await, BedStatus::Available);
    assert_eq!(
        bed_status(&s, open).await,
        BedStatus::Reserved,
        "no hold time, no expiry"
    );
    let enc_status =
        s.db.one(Stmt::new(
            "SELECT status FROM encounters WHERE id = $1::uuid",
            [enc.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(enc_status.col::<String>("status").unwrap(), "cancelled");
    assert_eq!(events(&app, held).await, ["reserved", "expired"]);
    assert_eq!(events(&app, open).await, ["reserved"]);

    // A second pass does not release it again, and the bed can be held again.
    s.adt.release_expired_holds(later).await.unwrap();
    assert_eq!(events(&app, held).await, ["reserved", "expired"]);
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": person, "bed_id": held, "hold_minutes": 5}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        events(&app, held).await,
        ["reserved", "expired", "reserved"]
    );
}

/// Cancelling a pre-admission closes its hold, so the sweep has nothing to do for it; a bed reserved with no
/// reservation row (before OFH-4) still cancels.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancelled_pre_admission_closes_its_hold() {
    let (s, app) = world().await;
    let b = beds(&app, 1).await[0];
    let person = Uuid::new_v4();
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": person, "bed_id": b, "hold_minutes": 10}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(
        &app,
        "POST",
        "/api/admissions/cancel-pre-admit",
        serde_json::json!({"person_id": person, "bed_id": b}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(bed_status(&s, b).await, BedStatus::Available);
    assert_eq!(events(&app, b).await, ["reserved", "cancelled"]);
    s.adt
        .release_expired_holds(Utc::now() + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(
        events(&app, b).await,
        ["reserved", "cancelled"],
        "already closed: the sweep adds nothing"
    );

    // A reservation made before the table existed has no row; the cancel still works.
    let person = Uuid::new_v4();
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": person, "bed_id": b}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    s.db.run(Stmt::new(
        "DELETE FROM bed_occupancy_events WHERE bed_id = $1",
        [b.into()],
    ))
    .await
    .unwrap();
    s.db.run(Stmt::new(
        "DELETE FROM bed_reservations WHERE bed_id = $1 AND released_at IS NULL",
        [b.into()],
    ))
    .await
    .unwrap();
    let (st, v) = call(
        &app,
        "POST",
        "/api/admissions/cancel-pre-admit",
        serde_json::json!({"person_id": person, "bed_id": b}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(bed_status(&s, b).await, BedStatus::Available);
}

/// A hold time out of range is refused and nothing is reserved; a bed has one open hold.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bad_hold_time_is_refused_and_a_bed_has_one_hold() {
    let (s, app) = world().await;
    let b = beds(&app, 1).await[0];
    for bad in [0, -5, 10_081] {
        let (st, _) = call(
            &app,
            "POST",
            "/api/admissions/pre-admit",
            serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": b, "hold_minutes": bad}),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(bed_status(&s, b).await, BedStatus::Available);
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": b, "hold_minutes": 60}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        serde_json::json!({"person_id": Uuid::new_v4(), "bed_id": b, "hold_minutes": 60}),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "a reserved bed cannot be reserved again"
    );
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/beds/{b}/occupancy-events?limit=1"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
}

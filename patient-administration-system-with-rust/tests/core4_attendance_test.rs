//! CORE-4: attendance and location. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::Appointment;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn staff() -> UserContext {
    UserContext {
        user_id: Some("nurse.b".into()),
        user_ip: None,
        user_agent: None,
    }
}

async fn booked(s: &AppState) -> Appointment {
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 1).await;
    s.scheduling
        .book_slot(slot.id, Uuid::new_v4(), &staff())
        .await
        .unwrap()
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn check_in_and_complete_stamp_the_times() {
    let s = state().await;
    let a = booked(&s).await;
    assert!(a.arrived_at.is_none() && a.completed_at.is_none());
    let arrived = s.scheduling.check_in(a.id, &staff()).await.unwrap();
    let t1 = arrived.arrived_at.expect("stamped on arrival");
    assert!(arrived.completed_at.is_none());
    let done = s.scheduling.complete(a.id, &staff()).await.unwrap();
    assert_eq!(done.arrived_at, Some(t1), "the arrival time is kept");
    assert!(done.completed_at.unwrap() >= t1);

    // A miss stamps neither.
    let b = booked(&s).await;
    let missed = s.scheduling.mark_no_show(b.id, &staff()).await.unwrap();
    assert!(missed.arrived_at.is_none() && missed.completed_at.is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn moving_a_patient_updates_the_room_and_is_audited_with_where_they_came_from() {
    let s = state().await;
    let a = booked(&s).await;
    // Not before arrival.
    assert!(
        s.scheduling
            .move_to_room(a.id, Some("Room 4"), &staff())
            .await
            .is_err()
    );
    s.scheduling.check_in(a.id, &staff()).await.unwrap();
    let m1 = s
        .scheduling
        .move_to_room(a.id, Some("  Waiting area "), &staff())
        .await
        .unwrap();
    assert_eq!(m1.current_room.as_deref(), Some("Waiting area"));
    let m2 = s
        .scheduling
        .move_to_room(a.id, Some("Room 4"), &staff())
        .await
        .unwrap();
    assert_eq!(m2.current_room.as_deref(), Some("Room 4"));
    assert_eq!(common::audit_count(&s.db, a.id, "moved").await, 2);
    let h = s.history.appointment_history(a.id).await.unwrap();
    let last = h
        .iter()
        .rev()
        .find(|e| e.action.as_deref() == Some("moved"))
        .unwrap();
    assert_eq!(last.by.as_deref(), Some("nurse.b"));
    assert_eq!(last.reason_code.as_deref(), Some("moved_room"));

    // An empty room clears it; a bad one is refused and changes nothing.
    assert_eq!(
        s.scheduling
            .move_to_room(a.id, Some(""), &staff())
            .await
            .unwrap()
            .current_room,
        None
    );
    assert!(
        s.scheduling
            .move_to_room(a.id, Some(&"x".repeat(61)), &staff())
            .await
            .is_err()
    );
    assert!(
        s.scheduling
            .move_to_room(a.id, Some("bad\nroom"), &staff())
            .await
            .is_err()
    );
    // Not once they have gone.
    s.scheduling
        .move_to_room(a.id, Some("Room 4"), &staff())
        .await
        .unwrap();
    s.scheduling.complete(a.id, &staff()).await.unwrap();
    assert!(
        s.scheduling
            .move_to_room(a.id, Some("Room 5"), &staff())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_in_facility_list_shows_minutes_waiting_longest_first_and_drops_the_completed() {
    let s = state().await;
    let (early, late, gone) = (booked(&s).await, booked(&s).await, booked(&s).await);
    for a in [&early, &late, &gone] {
        s.scheduling.check_in(a.id, &staff()).await.unwrap();
    }
    // Backdate their arrivals: 50, 5 and 20 minutes ago.
    for (a, mins) in [(&early, 50), (&late, 5), (&gone, 20)] {
        s.db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE appointments SET arrived_at = now() - make_interval(mins => $2::int) WHERE id = $1",
            [a.id.into(), mins.into()],
        ))
        .await
        .unwrap();
    }
    s.scheduling
        .move_to_room(early.id, Some("Room 2"), &staff())
        .await
        .unwrap();
    s.scheduling.complete(gone.id, &staff()).await.unwrap();

    let list = s.scheduling.in_facility().await.unwrap();
    let ours: Vec<&serde_json::Value> = list
        .iter()
        .filter(|e| {
            [early.id, late.id, gone.id]
                .iter()
                .any(|id| e["appointment_id"] == id.to_string())
        })
        .collect();
    assert_eq!(ours.len(), 2, "the completed one has gone");
    assert_eq!(
        ours[0]["appointment_id"],
        early.id.to_string(),
        "longest waiting first"
    );
    let w0 = ours[0]["waiting_minutes"].as_i64().unwrap();
    let w1 = ours[1]["waiting_minutes"].as_i64().unwrap();
    assert!((49..=51).contains(&w0), "{w0}");
    assert!((4..=6).contains(&w1), "{w1}");
    assert_eq!(ours[0]["room"], "Room 2");
    assert!(ours[1]["room"].is_null());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_move_and_list() {
    let s = state().await;
    let a = booked(&s).await;
    s.scheduling.check_in(a.id, &staff()).await.unwrap();
    let app = patient_administration_system::api::rest::router(s.clone());
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (st, v) = call(
        "PUT",
        format!("/api/appointments/{}/location", a.id),
        Some(serde_json::json!({ "room": "Room 9" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["current_room"], "Room 9");
    let (st, v) = call("GET", "/api/facility/in-facility".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["appointment_id"] == a.id.to_string() && e["room"] == "Room 9")
    );
    let (st, _) = call(
        "PUT",
        format!("/api/appointments/{}/location", Uuid::new_v4()),
        Some(serde_json::json!({ "room": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let after = AppointmentRepository::find_by_id(&s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.current_room.as_deref(), Some("Room 9"));
}

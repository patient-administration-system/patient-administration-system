//! TSC-10: a clinician's or room's absence makes work, not cancellations. Needs PostgreSQL:
//! `cargo test -- --ignored --test-threads=1`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::absences::{AbsenceService, AbsentKind, NewAbsence};
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use tower::ServiceExt;
use uuid::Uuid;

fn ctx() -> UserContext {
    UserContext {
        user_id: Some("Rota".into()),
        user_ip: None,
        user_agent: None,
    }
}

/// A day of its own, well in the future, so no other test's slots fall in the window.
fn day() -> Timestamp {
    let h = i64::try_from(Uuid::new_v4().as_u128() % 50_000).unwrap();
    Timestamp::now() + SignedDuration::from_hours(24 * 400 + h * 24)
}

async fn schedule(db: &Pool, kind: &str, owner: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO schedules (id, owner_kind, owner_id, service_type, active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'cardiology', true, now(), now())",
        [id.into(), kind.into(), owner.into()],
    ))
    .await
    .unwrap();
    id
}

async fn slot(db: &Pool, schedule: Uuid, start: Timestamp, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now(), now())",
        [id.into(), schedule.into(), start.into(), (start + SignedDuration::from_mins(30)).into(), status.into()],
    ))
    .await
    .unwrap();
    id
}

async fn booked(
    db: &Pool,
    start: Timestamp,
    slot: Option<Uuid>,
    practitioner: Option<Uuid>,
) -> Uuid {
    let mut a = Appointment::new(Uuid::new_v4(), start, start + SignedDuration::from_mins(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(db, &a).await.unwrap();
    db.run(Stmt::new(
        "UPDATE appointments SET slot_id = $2, worker_id = $3 WHERE id = $1",
        [a.id.into(), slot.into(), practitioner.into()],
    ))
    .await
    .unwrap();
    a.id
}

async fn status(db: &Pool, slot: Uuid) -> String {
    db.one(Stmt::new(
        "SELECT status FROM slots WHERE id = $1",
        [slot.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .col("status")
    .unwrap()
}

/// Every column of these appointments, to prove none changed.
async fn snapshot(db: &Pool, ids: &[Uuid]) -> Vec<serde_json::Value> {
    db.all(Stmt::new(
        "SELECT to_jsonb(a) AS row FROM appointments a WHERE id = ANY($1) ORDER BY id",
        [ids.to_vec().into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| r.col("row").unwrap())
    .collect()
}

/// The absence_rebook items for an absence: (appointment, status, priority, due).
async fn items(db: &Pool, absence: Uuid) -> Vec<(Uuid, String, i16, Timestamp)> {
    db.all(Stmt::new(
        "SELECT w.subject_id, w.status, w.priority, w.due_at FROM work_items w \
         JOIN absence_appointments aa ON aa.id = w.source_id \
         WHERE w.kind = 'absence_rebook' AND aa.absence_id = $1 ORDER BY w.subject_id",
        [absence.into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.col("subject_id").unwrap(),
            r.col("status").unwrap(),
            r.col("priority").unwrap(),
            r.col("due_at").unwrap(),
        )
    })
    .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_absence_over_two_booked_appointments_makes_two_items_blocks_the_free_slots_and_changes_no_appointment()
 {
    let db = common::build_state(&common::database_url().unwrap())
        .await
        .db;
    let svc = AbsenceService::new(db.clone());
    let (clinician, other) = (Uuid::new_v4(), Uuid::new_v4());
    let (mine, theirs) = (
        schedule(&db, "practitioner", clinician).await,
        schedule(&db, "practitioner", other).await,
    );
    let t = day();
    let h = |n: i64| t + SignedDuration::from_hours(n);
    let free1 = slot(&db, mine, h(1), "free").await;
    let free2 = slot(&db, mine, h(2), "free").await;
    let taken = slot(&db, mine, h(3), "busy").await;
    let outside = slot(&db, mine, h(30), "free").await;
    let someone_elses = slot(&db, theirs, h(1), "free").await;
    let on_slot = booked(&db, h(3), Some(taken), None).await;
    let by_clinician = booked(&db, h(4), None, Some(clinician)).await;
    let not_theirs = booked(&db, h(4), None, Some(other)).await;
    let all = [on_slot, by_clinician, not_theirs];
    let before = snapshot(&db, &all).await;

    let absence = svc
        .record(
            NewAbsence {
                owner_kind: AbsentKind::Practitioner,
                owner_id: clinician,
                starts_at: t,
                ends_at: h(8),
            },
            "Rota",
            &ctx(),
        )
        .await
        .unwrap();
    let mut expected = vec![on_slot, by_clinician];
    expected.sort();
    assert_eq!(
        absence.appointments, expected,
        "the two booked appointments it touches"
    );
    let mut blocked = vec![free1, free2];
    blocked.sort();
    assert_eq!(
        absence.blocked_slots, blocked,
        "the free slots in the window, no others"
    );
    assert_eq!(status(&db, free1).await, "blocked_out");
    assert_eq!(status(&db, taken).await, "busy");
    assert_eq!(status(&db, outside).await, "free");
    assert_eq!(status(&db, someone_elses).await, "free");
    assert_eq!(common::audit_count(&db, absence.id, "record").await, 1);

    let open = items(&db, absence.id).await;
    assert_eq!(open.len(), 2, "{open:?}");
    for (appt, st, priority, due) in &open {
        assert_eq!((st.as_str(), *priority), ("open", 1));
        let start = if *appt == on_slot { h(3) } else { h(4) };
        assert!(*due <= start, "due before the appointment");
    }
    assert_eq!(snapshot(&db, &all).await, before, "no appointment changed");

    // Withdrawn before anyone acts: the slots are free again, the items close, still nothing changed.
    let w = svc.withdraw(absence.id, "Rota", &ctx()).await.unwrap();
    assert!(w.withdrawn_at.is_some());
    assert_eq!(status(&db, free1).await, "free");
    assert_eq!(status(&db, free2).await, "free");
    assert!(items(&db, absence.id).await.iter().all(|i| i.1 == "closed"));
    assert_eq!(snapshot(&db, &all).await, before, "no appointment changed");
    assert!(matches!(
        svc.withdraw(absence.id, "Rota", &ctx()).await,
        Err(patient_administration_system::Error::Conflict(_))
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_rebooked_appointment_closes_its_item_and_a_slot_two_absences_block_stays_blocked_until_both_go()
 {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let db = state.db.clone();
    let svc = AbsenceService::new(db.clone());
    let room = Uuid::new_v4();
    let sched = schedule(&db, "room", room).await;
    let t = day();
    let h = |n: i64| t + SignedDuration::from_hours(n);
    let free = slot(&db, sched, h(2), "free").await;
    let taken = slot(&db, sched, h(3), "busy").await;
    let appt = booked(&db, h(3), Some(taken), None).await;
    let new = |s, e| NewAbsence {
        owner_kind: AbsentKind::Room,
        owner_id: room,
        starts_at: s,
        ends_at: e,
    };
    let first = svc.record(new(t, h(5)), "Rota", &ctx()).await.unwrap();
    let second = svc.record(new(h(1), h(6)), "Rota", &ctx()).await.unwrap();
    assert_eq!(first.blocked_slots, vec![free]);
    assert!(
        second.blocked_slots.is_empty(),
        "already blocked by the first"
    );

    // A person rebooked the patient elsewhere (in the appointment's own place): its item closes.
    db.run(Stmt::new("UPDATE appointments SET start_datetime = start_datetime + interval '30 days', end_datetime = end_datetime + interval '30 days' WHERE id = $1", [appt.into()]))
        .await
        .unwrap();
    patient_administration_system::work_queue::WorkQueueService::new(db.clone())
        .sync(Timestamp::now())
        .await
        .unwrap();
    assert!(items(&db, first.id).await.iter().all(|i| i.1 == "closed"));

    // The slot stays blocked while another absence covers it... (the first blocked it; withdrawing the
    // second does not free a slot it never blocked)
    svc.withdraw(second.id, "Rota", &ctx()).await.unwrap();
    assert_eq!(status(&db, free).await, "blocked_out");
    svc.withdraw(first.id, "Rota", &ctx()).await.unwrap();
    assert_eq!(status(&db, free).await, "free");

    // Through the API: a window that ends before it starts, or with no signed-in person, is refused.
    let app = patient_administration_system::api::rest::router(state);
    let body = |s: Timestamp, e: Timestamp| {
        serde_json::json!({"owner_kind": "room", "owner_id": room, "starts_at": s, "ends_at": e})
            .to_string()
    };
    let post = |b: String, who: Option<&'static str>| {
        let mut r = Request::post("/api/absences").header("content-type", "application/json");
        if let Some(w) = who {
            r = r.header("x-user-id", w);
        }
        app.clone().oneshot(r.body(Body::from(b)).unwrap())
    };
    assert_eq!(
        post(body(h(5), h(1)), Some("Rota")).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert!(
        post(body(h(1), h(5)), None)
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    let ok = post(body(h(10), h(11)), Some("Rota")).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn absences_are_listed_by_owner_active_and_window_soonest_first() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = AbsenceService::new(state.db.clone());
    let owner = Uuid::new_v4();
    let t = day();
    let h = |n: i64| t + SignedDuration::from_hours(n);
    let new = |s, e| NewAbsence {
        owner_kind: AbsentKind::Practitioner,
        owner_id: owner,
        starts_at: s,
        ends_at: e,
    };
    let later = svc.record(new(h(48), h(50)), "Rota", &ctx()).await.unwrap();
    let sooner = svc.record(new(h(1), h(3)), "Rota", &ctx()).await.unwrap();
    let gone = svc.record(new(h(10), h(12)), "Rota", &ctx()).await.unwrap();
    svc.withdraw(gone.id, "Rota", &ctx()).await.unwrap();
    let app = patient_administration_system::api::rest::router(state);
    let list = |q: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::get(format!("/api/absences?owner_id={owner}{q}").replace('+', "%2B"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = r.status();
            let v: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap(),
            )
            .unwrap_or_default();
            (status, v)
        }
    };
    let ids = |v: &serde_json::Value| -> Vec<String> {
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (st, v) = list(String::new()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        ids(&v),
        vec![sooner.id.to_string(), later.id.to_string()],
        "active only, soonest first"
    );
    let (_, v) = list("&active=false".into()).await;
    assert_eq!(
        ids(&v),
        vec![
            sooner.id.to_string(),
            gone.id.to_string(),
            later.id.to_string()
        ]
    );
    let (_, v) = list(format!("&from={}&to={}", h(2), h(20))).await;
    assert_eq!(
        ids(&v),
        vec![sooner.id.to_string()],
        "overlapping the window"
    );
    let (st, _) = list(format!("&from={}&to={}", h(5), h(1))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

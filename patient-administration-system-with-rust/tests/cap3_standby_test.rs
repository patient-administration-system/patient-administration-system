//! CAP-3: released slots and the standby list. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{
    Appointment, BarrierCode, CancelledBy, ReleaseClass,
};
use patient_administration_system::scheduling::CancelDetails;
use patient_administration_system::standby::{CandidateFilter, StandbyInput};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

struct W {
    state: AppState,
    service: String,
    schedule: Uuid,
}

async fn world() -> W {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&state.db, &service).await.id;
    W {
        state,
        service,
        schedule,
    }
}

fn ctx() -> UserContext {
    UserContext::system()
}

async fn booked(w: &W, person: Uuid, in_days: i64) -> (Appointment, Uuid) {
    let slot = common::free_slot(&w.state.db, w.schedule, in_days).await;
    (
        w.state
            .scheduling
            .book_slot(slot.id, person, &ctx())
            .await
            .unwrap(),
        slot.id,
    )
}

async fn cancel(w: &W, a: &Appointment, barrier: Option<BarrierCode>) {
    w.state
        .scheduling
        .cancel_with(
            a.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: barrier,
                legacy_reason: None,
            },
            &ctx(),
        )
        .await
        .unwrap();
}

async fn release_for(w: &W, slot: Uuid) -> serde_json::Value {
    let row = w.state.db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT id::text AS id, status, release_class, notice_days, service_type, seconds_to_refill, refilled_by_appointment_id::text AS by FROM slot_releases WHERE slot_id = $1 ORDER BY released_at DESC LIMIT 1",
        [slot.into()],
    )).await.unwrap();
    match row {
        None => serde_json::Value::Null,
        Some(r) => serde_json::json!({
            "id": r.try_get::<String>("", "id").unwrap(),
            "status": r.try_get::<String>("", "status").unwrap(),
            "release_class": r.try_get::<String>("", "release_class").unwrap(),
            "notice_days": r.try_get::<Option<i32>>("", "notice_days").unwrap(),
            "service_type": r.try_get::<Option<String>>("", "service_type").unwrap(),
            "seconds_to_refill": r.try_get::<Option<i64>>("", "seconds_to_refill").unwrap(),
            "by": r.try_get::<Option<String>>("", "by").unwrap(),
        }),
    }
}

async fn event_count(w: &W, release_id: &str) -> i64 {
    let r = w.state.db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) AS n FROM outbox_events WHERE event_type = 'SlotReleased' AND payload->>'release_id' = $1",
        [release_id.into()],
    )).await.unwrap().unwrap();
    r.try_get("", "n").unwrap()
}

fn standby(person: Uuid, service: &str) -> StandbyInput {
    StandbyInput {
        person_id: person,
        service_type: service.into(),
        short_notice_ok: true,
        language: None,
        needs_transport: false,
        phone_only: false,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_records_one_release_with_its_class_and_publishes_one_event() {
    let w = world().await;
    let (a, slot) = booked(&w, Uuid::new_v4(), 6).await;
    cancel(&w, &a, Some(BarrierCode::Work)).await;
    let r = release_for(&w, slot).await;
    assert_eq!(r["status"], "open");
    assert_eq!(r["release_class"], "early_release");
    assert_eq!(r["notice_days"], 5);
    assert_eq!(r["service_type"], w.service.as_str());
    assert_eq!(
        event_count(&w, r["id"].as_str().unwrap()).await,
        1,
        "exactly one SlotReleased event"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_hours_before_is_a_late_release_and_a_miss_of_a_past_slot_releases_nothing() {
    let w = world().await;
    let late_slot = common::free_slot_in_hours(&w.state.db, w.schedule, 3).await;
    let a = w
        .state
        .scheduling
        .book_slot(late_slot.id, Uuid::new_v4(), &ctx())
        .await
        .unwrap();
    let slot = late_slot.id;
    cancel(&w, &a, None).await;
    assert_eq!(
        release_for(&w, slot).await["release_class"],
        "late_release",
        "three hours is no notice in whole days"
    );

    // A slot whose time has passed cannot be refilled, so a miss there records no release.
    let (past, past_slot) = booked(&w, Uuid::new_v4(), -1).await;
    w.state
        .scheduling
        .mark_no_show(past.id, &ctx())
        .await
        .unwrap();
    assert!(release_for(&w, past_slot).await.is_null());

    // A miss of a still-future slot (marked early) is a silent miss and is recorded.
    let (soon, soon_slot) = booked(&w, Uuid::new_v4(), 1).await;
    w.state
        .scheduling
        .mark_no_show(soon.id, &ctx())
        .await
        .unwrap();
    assert_eq!(
        release_for(&w, soon_slot).await["release_class"],
        ReleaseClass::SilentMiss.to_string_for_test()
    );
}

trait Wire {
    fn to_string_for_test(&self) -> String;
}
impl Wire for ReleaseClass {
    fn to_string_for_test(&self) -> String {
        serde_json::to_value(self)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn booking_the_released_slot_marks_it_refilled_with_how_fast() {
    let w = world().await;
    let (a, slot) = booked(&w, Uuid::new_v4(), 6).await;
    cancel(&w, &a, None).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let taker = Uuid::new_v4();
    let again = w
        .state
        .scheduling
        .book_slot(slot, taker, &ctx())
        .await
        .unwrap();
    let r = release_for(&w, slot).await;
    assert_eq!(r["status"], "refilled");
    assert_eq!(r["by"], again.id.to_string());
    assert!(
        r["seconds_to_refill"].as_i64().unwrap() >= 1,
        "it took at least the second we waited: {r}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn candidates_are_longest_waiting_first_and_exclude_the_unavailable() {
    let w = world().await;
    let canceller = Uuid::new_v4();
    let (a, slot) = booked(&w, canceller, 6).await;
    let (first, second, short_notice_no, clashing, wrong_service, inactive) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    for p in [first, second] {
        w.state
            .standby
            .add(standby(p, &w.service), &ctx())
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut no = standby(short_notice_no, &w.service);
    no.short_notice_ok = false;
    w.state.standby.add(no, &ctx()).await.unwrap();
    w.state
        .standby
        .add(standby(canceller, &w.service), &ctx())
        .await
        .unwrap(); // the one who gave the slot up
    w.state
        .standby
        .add(standby(clashing, &w.service), &ctx())
        .await
        .unwrap();
    w.state
        .standby
        .add(standby(wrong_service, "some-other-service"), &ctx())
        .await
        .unwrap();
    let gone = w
        .state
        .standby
        .add(standby(inactive, &w.service), &ctx())
        .await
        .unwrap();
    w.state.standby.deactivate(gone.id, &ctx()).await.unwrap();
    // `clashing` already has an appointment at that time in another schedule.
    let other = common::new_schedule_for(&w.state.db, "elsewhere").await;
    let slot_row =
        patient_administration_system::db::repositories::slot::SlotRepository::find_by_id(
            &w.state.db,
            slot,
        )
        .await
        .unwrap()
        .unwrap();
    let clash_slot = patient_administration_system::models::schedule::Slot::new(
        other.id,
        slot_row.start_datetime,
        slot_row.end_datetime,
    );
    let clash_slot = patient_administration_system::db::repositories::slot::SlotRepository::create(
        &w.state.db,
        &clash_slot,
    )
    .await
    .unwrap();
    w.state
        .scheduling
        .book_slot(clash_slot.id, clashing, &ctx())
        .await
        .unwrap();

    cancel(&w, &a, None).await;
    let rel = release_for(&w, slot).await;
    let id: Uuid = rel["id"].as_str().unwrap().parse().unwrap();
    let got: Vec<Uuid> = w
        .state
        .standby
        .candidates(id, &CandidateFilter::default())
        .await
        .unwrap()
        .iter()
        .map(|e| e.person_id)
        .collect();
    assert_eq!(
        got,
        vec![first, second],
        "longest on standby first; nobody unavailable"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn tag_filters_narrow_candidates_without_reordering_them() {
    let w = world().await;
    let (a, slot) = booked(&w, Uuid::new_v4(), 6).await;
    let (welsh, english, welsh_phone) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut i = standby(welsh, &w.service);
    i.language = Some("cy-001".into());
    w.state.standby.add(i, &ctx()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    w.state
        .standby
        .add(standby(english, &w.service), &ctx())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let mut j = standby(welsh_phone, &w.service);
    j.language = Some("cy-001".into());
    j.phone_only = true;
    w.state.standby.add(j, &ctx()).await.unwrap();
    cancel(&w, &a, None).await;
    let id: Uuid = release_for(&w, slot).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let ids = |f: CandidateFilter| {
        let s = w.state.standby.clone();
        async move {
            s.candidates(id, &f)
                .await
                .unwrap()
                .iter()
                .map(|e| e.person_id)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        ids(CandidateFilter {
            language: Some("cy-001".into()),
            ..Default::default()
        })
        .await,
        vec![welsh, welsh_phone]
    );
    assert_eq!(
        ids(CandidateFilter {
            phone_only: Some(true),
            ..Default::default()
        })
        .await,
        vec![welsh_phone]
    );
    assert_eq!(
        ids(CandidateFilter::default()).await,
        vec![welsh, english, welsh_phone]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_refilled_release_has_no_candidates_and_booking_from_standby_is_all_or_nothing() {
    let w = world().await;
    let (a, slot) = booked(&w, Uuid::new_v4(), 6).await;
    let patient = Uuid::new_v4();
    let entry = w
        .state
        .standby
        .add(standby(patient, &w.service), &ctx())
        .await
        .unwrap();
    cancel(&w, &a, None).await;
    let id: Uuid = release_for(&w, slot).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let appt = w
        .state
        .standby
        .book_from_standby(entry.id, slot, &ctx())
        .await
        .unwrap();
    assert_eq!(appt.person_id, patient);
    assert_eq!(release_for(&w, slot).await["status"], "refilled");
    assert!(
        w.state
            .standby
            .list(Some(&w.service))
            .await
            .unwrap()
            .is_empty(),
        "taken off the list"
    );
    assert!(
        w.state
            .standby
            .candidates(id, &CandidateFilter::default())
            .await
            .is_err(),
        "nothing left to offer"
    );

    // A booking that fails (slot taken) leaves the patient on standby.
    let other = Uuid::new_v4();
    let still = w
        .state
        .standby
        .add(standby(other, &w.service), &ctx())
        .await
        .unwrap();
    assert!(
        w.state
            .standby
            .book_from_standby(still.id, slot, &ctx())
            .await
            .is_err()
    );
    assert_eq!(
        w.state.standby.list(Some(&w.service)).await.unwrap().len(),
        1,
        "still on standby after the failed booking"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn adding_twice_updates_the_tags_instead_of_duplicating() {
    let w = world().await;
    let p = Uuid::new_v4();
    w.state
        .standby
        .add(standby(p, &w.service), &ctx())
        .await
        .unwrap();
    let mut again = standby(p, &w.service);
    again.needs_transport = true;
    let e = w.state.standby.add(again, &ctx()).await.unwrap();
    assert!(e.needs_transport);
    assert_eq!(
        w.state.standby.list(Some(&w.service)).await.unwrap().len(),
        1
    );
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let s = r.status();
    (
        s,
        serde_json::from_slice(&to_bytes(r.into_body(), 4 << 20).await.unwrap())
            .unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_booking_team_sees_open_refilled_and_expired_releases_with_a_summary() {
    let w = world().await;
    let app = patient_administration_system::api::rest::router(w.state.clone());
    let (open_a, _open_slot) = booked(&w, Uuid::new_v4(), 6).await;
    let (filled_a, filled_slot) = booked(&w, Uuid::new_v4(), 6).await;
    cancel(&w, &open_a, None).await;
    cancel(&w, &filled_a, None).await;
    w.state
        .scheduling
        .book_slot(filled_slot, Uuid::new_v4(), &ctx())
        .await
        .unwrap();

    // Make one release's slot time pass without refilling it.
    let (stale_a, stale_slot) = booked(&w, Uuid::new_v4(), 2).await;
    cancel(&w, &stale_a, None).await;
    w.state.db.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "UPDATE slot_releases SET start_datetime = now() - interval '1 hour' WHERE slot_id = $1", [stale_slot.into()])).await.unwrap();

    let (s, v) = get(app.clone(), "/api/slot-releases").await;
    assert_eq!(s, StatusCode::OK);
    let mine: Vec<&serde_json::Value> = v["data"]["releases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["service_type"] == w.service.as_str())
        .collect();
    let status_of = |slot: Uuid| {
        mine.iter()
            .find(|r| r["slot_id"] == slot.to_string())
            .map(|r| r["status"].clone())
    };
    assert_eq!(status_of(filled_slot), Some("refilled".into()));
    assert_eq!(
        status_of(stale_slot),
        Some("expired".into()),
        "derived from the start time, not stored"
    );
    assert!(mine.iter().any(|r| r["status"] == "open"));
    assert!(v["data"]["summary"]["total"].as_i64().unwrap() >= 3);

    let (s, only) = get(app.clone(), "/api/slot-releases?status=refilled").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        only["data"]["releases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "refilled")
    );
    let (s, _) = get(app, "/api/slot-releases?status=bogus").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

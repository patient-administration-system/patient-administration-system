//! NEW-10: IHE ITI Scheduling `$find`, `$hold` and `$book` over the existing booking rules. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::schedule::{ScheduleOwner, Slot};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

fn at(days: i64, hour: u32) -> DateTime<Utc> {
    let d = (Utc::now() + Duration::days(days)).date_naive();
    Utc.with_ymd_and_hms(d.year(), d.month(), d.day(), hour, 0, 0)
        .unwrap()
}

async fn post(app: &axum::Router, op: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::post(format!("/fhir/r4/Appointment/{op}"))
                .header("content-type", "application/fhir+json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let ctype = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ctype.contains("fhirVersion=4.0"), "{ctype}");
    (
        status,
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap(),
    )
}

fn find_body(patient: Uuid, practitioner: Uuid, days: (i64, i64)) -> Value {
    json!({"resourceType": "Parameters", "parameter": [
        {"name": "start", "valueDateTime": at(days.0, 0)},
        {"name": "end", "valueDateTime": at(days.1, 0)},
        {"name": "practitioner", "valueReference": {"reference": format!("Practitioner/{practitioner}")}},
        {"name": "patient-reference", "valueReference": {"reference": format!("Patient/{patient}")}},
    ]})
}

fn reference(id: &str) -> Value {
    json!({"resourceType": "Parameters", "parameter": [
        {"name": "appointment-reference", "valueReference": {"reference": format!("Appointment/{id}")}},
    ]})
}

fn ids(bundle: &Value) -> Vec<String> {
    bundle["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
        .collect()
}

fn slot_of(bundle: &Value, i: usize) -> String {
    bundle["entry"][i]["resource"]["slot"][0]["reference"]
        .as_str()
        .unwrap()
        .trim_start_matches("Slot/")
        .to_string()
}

async fn both(db: &sea_orm::DatabaseConnection, id: Uuid) -> (String, bool, i64, String) {
    let r = sea_orm::ConnectionTrait::query_one_raw(
            db,
            sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Postgres,
                "SELECT a.status, (a.end_datetime - a.start_datetime) AS len, a.slot_id IS NOT NULL AS has_slot, \
                        (SELECT count(*) FROM audit_log WHERE entity_type = 'appointment' AND entity_id = a.id AND action = 'book') AS audited, \
                        (SELECT status FROM slots WHERE id = a.slot_id) AS slot_status \
                 FROM appointments a WHERE a.id = $1",
                [id.into()],
            ),
        )
        .await
        .unwrap()
        .unwrap();
    (
        r.try_get::<String>("", "status").unwrap(),
        r.try_get::<bool>("", "has_slot").unwrap(),
        r.try_get::<i64>("", "audited").unwrap(),
        r.try_get::<String>("", "slot_status").unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn find_hold_and_book_follow_the_booking_rules_and_a_hold_expires() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = fhir_router(state.clone());
    let sys = UserContext::system();
    let sched = common::new_schedule(&state.db).await;
    let ScheduleOwner::Practitioner(doctor) = sched.owner else {
        panic!("a practitioner schedule");
    };
    let mut slots = Vec::new();
    for hour in [9, 10, 11, 12] {
        slots.push(
            SlotRepository::create(
                &state.db,
                &Slot::new(sched.id, at(30, hour), at(30, hour) + Duration::minutes(15)),
            )
            .await
            .unwrap(),
        );
    }
    let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

    // $find: the four slots, earliest first, each proposed to the patient.
    let (status, found) = post(&app, "$find", find_body(a, doctor, (29, 32))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found["resourceType"], "Bundle");
    assert_eq!(found["total"], 4);
    let order: Vec<_> = (0..4).map(|i| slot_of(&found, i)).collect();
    let expected: Vec<_> = slots.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(order, expected, "time order and nothing else");
    assert_eq!(found["entry"][0]["resource"]["status"], "proposed");
    let first = ids(&found)[0].clone();

    // $hold: pending, and nobody else can book or be offered that slot.
    let (status, held) = post(&app, "$hold", reference(&first)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held["entry"][0]["resource"]["status"], "pending");
    let (_, for_b) = post(&app, "$find", find_body(b, doctor, (29, 32))).await;
    assert_eq!(for_b["total"], 3, "the held slot is left out for B");
    assert!(!(0..3).any(|i| slot_of(&for_b, i) == slots[0].id.to_string()));
    let native = state.scheduling.book_slot(slots[0].id, b, &sys).await;
    assert!(native.is_err(), "the native route respects the hold");

    // $book: booked, and the same as a native booking of another slot.
    let (status, booked) = post(&app, "$book", reference(&first)).await;
    assert_eq!(status, StatusCode::OK);
    let appt = &booked["entry"][0]["resource"];
    assert_eq!(appt["status"], "booked");
    let ihe_id: Uuid = appt["id"].as_str().unwrap().parse().unwrap();
    let native = state
        .scheduling
        .book_slot(slots[1].id, c, &sys)
        .await
        .unwrap();
    assert_eq!(
        both(&state.db, ihe_id).await,
        both(&state.db, native.id).await
    );
    assert_eq!(both(&state.db, ihe_id).await.0, "booked");
    // Booking it again, or holding it, is refused as no longer available or already booked.
    let (status, _) = post(&app, "$book", reference(&first)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, gone) = post(&app, "$hold", reference(&first)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(gone["issue"][0]["severity"], "fatal");
    assert_eq!(gone["issue"][0]["code"], "not-found");

    // A hold that has run out lets someone else book the slot.
    let (_, again) = post(&app, "$find", find_body(b, doctor, (29, 32))).await;
    assert_eq!(again["total"], 2, "slots 3 and 4");
    let lapsed: Uuid = ids(&again)[0].parse().unwrap();
    let lapsed_slot: Uuid = slot_of(&again, 0).parse().unwrap();
    state
        .scheduling
        .hold_proposal(
            lapsed,
            Utc::now() - Duration::minutes(20),
            Duration::minutes(10),
            &sys,
        )
        .await
        .unwrap();
    let taken = state.scheduling.book_slot(lapsed_slot, c, &sys).await;
    assert!(taken.is_ok(), "the hold ran out ten minutes ago: {taken:?}");

    // Refusals name what is wrong.
    let bad = json!({"resourceType": "Parameters", "parameter": [
        {"name": "specialty", "valueString": "cardiology"},
        {"name": "patient-reference", "valueReference": {"reference": format!("Patient/{a}")}},
    ]});
    let (status, out) = post(&app, "$find", bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        out["issue"][0]["diagnostics"]
            .as_str()
            .unwrap()
            .contains("specialty")
    );
    let (status, _) = post(
        &app,
        "$find",
        json!({"resourceType": "Parameters", "parameter": []}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a patient is required");
    let (status, _) = post(&app, "$hold", reference(&Uuid::new_v4().to_string())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

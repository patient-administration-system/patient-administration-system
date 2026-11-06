//! FR-202 and FR-203: `POST /fhir` writes a `transaction` or `batch` Bundle. Needs PostgreSQL:
//! `cargo test -- --ignored --test-threads=1`, because it counts rows before and after a request.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::Address;
use patient_administration_system::models::facility::{Bed, BedStatus, Facility, Room, Ward};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn count(s: &AppState, sql: &str) -> i64 {
    s.db.one(Stmt::text(format!("SELECT count(*) AS n FROM {sql}")))
        .await
        .unwrap()
        .unwrap()
        .col::<i64>("n")
        .unwrap()
}

/// schedules, slots, audit rows and outbox events for them.
async fn rows(s: &AppState) -> [i64; 4] {
    [
        count(s, "schedules").await,
        count(s, "slots").await,
        count(s, "audit_log WHERE entity_type IN ('schedule', 'slot')").await,
        count(
            s,
            "outbox_events WHERE event_type IN ('ScheduleCreated', 'SlotCreated')",
        )
        .await,
    ]
}

async fn post(s: &AppState, body: Value) -> (StatusCode, Value) {
    let res = fhir_router(s.clone())
        .oneshot(
            Request::post("/fhir")
                .header("content-type", "application/fhir+json")
                .header("x-user-id", "bundle-test")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn schedule_entry(full_url: &str) -> Value {
    json!({
        "fullUrl": full_url,
        "resource": {
            "resourceType": "Schedule",
            "actor": [{ "reference": format!("Practitioner/{}", Uuid::new_v4()) }],
        },
        "request": { "method": "POST", "url": "Schedule" },
    })
}

fn slot_entry(reference: &str) -> Value {
    json!({
        "resource": {
            "resourceType": "Slot",
            "schedule": { "reference": reference },
            "status": "free",
            "start": "2030-01-07T09:00:00Z",
            "end": "2030-01-07T09:15:00Z",
        },
        "request": { "method": "POST", "url": "Slot" },
    })
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({ "resourceType": "Bundle", "type": kind, "entry": entries })
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_transaction_writes_a_schedule_and_the_slot_that_names_it_and_can_be_read_back() {
    let s = state().await;
    let before = rows(&s).await;
    // The Slot comes first: the order of the entries must not matter.
    let (status, body) = post(
        &s,
        bundle(
            "transaction",
            vec![slot_entry("urn:uuid:a"), schedule_entry("urn:uuid:a")],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["type"], "transaction-response");
    assert_eq!(rows(&s).await, before.zip_add([1, 1, 2, 2]));
    // Answers keep the order of the request.
    let entry = body["entry"].as_array().unwrap();
    assert_eq!(entry[0]["response"]["status"], "201 Created");
    let slot = entry[0]["response"]["location"].as_str().unwrap();
    let schedule = entry[1]["response"]["location"].as_str().unwrap();
    assert!(slot.starts_with("Slot/") && schedule.starts_with("Schedule/"));

    let res = fhir_router(s.clone())
        .oneshot(
            Request::get(format!("/fhir/{slot}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let read: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(read["schedule"]["reference"], schedule);

    let id = slot.trim_start_matches("Slot/");
    assert_eq!(
        count(
            &s,
            &format!(
                "audit_log WHERE entity_id = '{id}' AND action = 'create' AND user_id = 'bundle-test'"
            )
        )
        .await,
        1,
        "the caller is named in the audit row"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_transaction_that_fails_while_writing_leaves_nothing_behind() {
    let s = state().await;
    let before = rows(&s).await;
    // Passes every check made before writing, so the Schedule is inserted and then has to roll back.
    let missing = format!("Schedule/{}", Uuid::new_v4());
    let (status, body) = post(
        &s,
        bundle(
            "transaction",
            vec![schedule_entry("urn:uuid:a"), slot_entry(&missing)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome");
    assert!(body.to_string().contains("does not exist"), "{body}");
    assert_eq!(rows(&s).await, before);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_transaction_with_one_refused_entry_writes_nothing_and_names_it() {
    let s = state().await;
    let before = rows(&s).await;
    let patient = json!({
        "resource": { "resourceType": "Patient" },
        "request": { "method": "POST", "url": "Patient" },
    });
    let (status, body) = post(
        &s,
        bundle(
            "transaction",
            vec![
                schedule_entry("urn:uuid:a"),
                patient,
                slot_entry("urn:uuid:b"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let text = body.to_string();
    assert!(
        text.contains("entry[1]") && text.contains("person-service"),
        "{text}"
    );
    assert!(
        text.contains("entry[2]") && text.contains("urn:uuid:b"),
        "{text}"
    );
    assert_eq!(rows(&s).await, before);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_batch_answers_each_entry_and_keeps_the_ones_that_worked() {
    let s = state().await;
    let before = rows(&s).await;
    let (status, body) = post(
        &s,
        bundle(
            "batch",
            vec![
                schedule_entry("urn:uuid:a"),
                // A batch's entries may not depend on one another.
                slot_entry("urn:uuid:a"),
                json!({
                    "resource": { "resourceType": "Encounter" },
                    "request": { "method": "POST", "url": "Encounter" },
                }),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["type"], "batch-response");
    let entry = body["entry"].as_array().unwrap();
    assert_eq!(entry[0]["response"]["status"], "201 Created");
    assert_eq!(entry[1]["response"]["status"], "400 Bad Request");
    assert_eq!(entry[2]["response"]["status"], "501 Not Implemented");
    assert_eq!(
        entry[2]["response"]["outcome"]["resourceType"],
        "OperationOutcome"
    );
    assert_eq!(rows(&s).await, before.zip_add([1, 0, 1, 1]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn what_is_not_a_write_bundle_is_refused() {
    let s = state().await;
    let before = rows(&s).await;
    for body in [
        json!({ "resourceType": "Patient" }),
        json!({ "resourceType": "Bundle", "type": "transaction", "entry": [] }),
        bundle("collection", vec![schedule_entry("urn:uuid:a")]),
    ] {
        let (status, outcome) = post(&s, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{outcome}");
    }
    assert_eq!(rows(&s).await, before);
}

fn coverage_entry() -> Value {
    json!({
        "resource": {
            "resourceType": "Coverage",
            "status": "active",
            "kind": "insurance",
            "beneficiary": { "reference": format!("Patient/{}", Uuid::new_v4()) },
            "insurer": { "display": "Bundle Mutual" },
            "identifier": [{ "value": "POL-1" }],
            "period": { "start": "2030-01-01", "end": "2030-12-31" },
        },
        "request": { "method": "POST", "url": "Coverage" },
    })
}

fn booking_entry(slot: &str) -> Value {
    json!({
        "resource": {
            "resourceType": "Appointment",
            "status": "booked",
            "subject": { "reference": format!("Patient/{}", Uuid::new_v4()) },
            "slot": [{ "reference": slot }],
        },
        "request": { "method": "POST", "url": "Appointment" },
    })
}

fn admission_entry(bed: Uuid) -> Value {
    json!({
        "resource": {
            "resourceType": "Encounter",
            "status": "in-progress",
            "class": [{ "coding": [{ "code": "IMP" }] }],
            "subject": { "reference": format!("Patient/{}", Uuid::new_v4()) },
            "location": [{ "location": { "reference": format!("Location/{bed}") } }],
        },
        "request": { "method": "POST", "url": "Encounter" },
    })
}

/// schedules, slots, coverages, appointments, encounters and admissions.
async fn records(s: &AppState) -> [i64; 6] {
    [
        count(s, "schedules").await,
        count(s, "slots").await,
        count(s, "coverages").await,
        count(s, "appointments").await,
        count(s, "encounters").await,
        count(s, "admissions").await,
    ]
}

async fn read(s: &AppState, location: &str) -> Value {
    let res = fhir_router(s.clone())
        .oneshot(
            Request::get(format!("/fhir/{location}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{location}");
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
}

async fn one_available_bed(s: &AppState) -> Uuid {
    let app = patient_administration_system::api::rest::router(s.clone());
    let call = |uri: &'static str, body: Value| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(
                    Request::post(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(res.status().is_success(), "{uri}: {}", res.status());
        }
    };
    let u = Uuid::new_v4().simple().to_string();
    let f = Facility::new("F".into(), format!("F{u}"), Address::default());
    call("/api/facilities", json!(f)).await;
    let w = Ward::new(f.id, "W".into(), format!("W{u}"), 4);
    call("/api/wards", json!(w)).await;
    let r = Room::new(w.id, "R".into(), format!("R{u}"));
    call("/api/rooms", json!(r)).await;
    let b = Bed::new(r.id, "B".into(), format!("B{u}"));
    call("/api/beds", json!(b)).await;
    b.id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_transaction_creates_a_clinic_and_books_into_it_whatever_the_order() {
    let s = state().await;
    let before = records(&s).await;
    let booked_events = count(&s, "outbox_events WHERE event_type = 'CoverageCreated'").await;
    let mut slot = slot_entry("urn:uuid:a");
    slot["fullUrl"] = json!("urn:uuid:b");
    let (status, body) = post(
        &s,
        bundle(
            "transaction",
            vec![
                booking_entry("urn:uuid:b"),
                coverage_entry(),
                slot,
                schedule_entry("urn:uuid:a"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        records(&s).await,
        std::array::from_fn(|i| before[i] + [1, 1, 1, 1, 0, 0][i])
    );
    let at = |i: usize| body["entry"][i]["response"]["location"].as_str().unwrap();
    let appointment = read(&s, at(0)).await;
    assert_eq!(appointment["status"], "booked");
    assert_eq!(appointment["slot"][0]["reference"], at(2));
    assert_eq!(read(&s, at(2)).await["status"], "busy");
    let coverage = read(&s, at(1)).await;
    assert_eq!(coverage["insurer"]["display"], "Bundle Mutual");
    assert_eq!(coverage["period"]["end"], "2030-12-31");
    assert_eq!(
        count(&s, "outbox_events WHERE event_type = 'CoverageCreated'").await,
        booked_events + 1
    );
    let id = at(0).trim_start_matches("Appointment/");
    assert!(
        count(&s, &format!("audit_log WHERE entity_id = '{id}'")).await >= 1,
        "the booking is audited"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_booking_refused_by_its_rules_rolls_the_whole_transaction_back() {
    let s = state().await;
    let before = records(&s).await;
    let mut slot = slot_entry("urn:uuid:a");
    slot["fullUrl"] = json!("urn:uuid:b");
    // Two patients into one slot: the second meets the same refusal as `POST /api/slots/{id}/book`.
    let (status, body) = post(
        &s,
        bundle(
            "transaction",
            vec![
                schedule_entry("urn:uuid:a"),
                slot,
                coverage_entry(),
                booking_entry("urn:uuid:b"),
                booking_entry("urn:uuid:b"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(records(&s).await, before);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn an_encounter_admits_to_the_bed_and_an_occupied_bed_refuses_the_whole_transaction() {
    let s = state().await;
    let bed = one_available_bed(&s).await;
    let before = records(&s).await;
    let (status, body) = post(&s, bundle("transaction", vec![admission_entry(bed)])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        records(&s).await,
        std::array::from_fn(|i| before[i] + [0, 0, 0, 0, 1, 1][i])
    );
    let location = body["entry"][0]["response"]["location"].as_str().unwrap();
    assert_eq!(read(&s, location).await["status"], "in-progress");
    let held =
        patient_administration_system::db::repositories::bed::BedRepository::find_by_id(&s.db, bed)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(held.status, BedStatus::Occupied);

    let before = records(&s).await;
    let (status, body) = post(
        &s,
        bundle("transaction", vec![coverage_entry(), admission_entry(bed)]),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(records(&s).await, before, "the Coverage went with it");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_batch_keeps_the_first_booking_and_answers_the_second() {
    let s = state().await;
    let schedule = common::new_schedule(&s.db).await;
    let slot = format!("Slot/{}", common::free_slot(&s.db, schedule.id, 5).await.id);
    let (status, body) = post(
        &s,
        bundle("batch", vec![booking_entry(&slot), booking_entry(&slot)]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["entry"][0]["response"]["status"], "201 Created");
    assert_eq!(body["entry"][1]["response"]["status"], "409 Conflict");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn an_entry_that_cannot_say_what_it_means_is_refused() {
    let s = state().await;
    let before = records(&s).await;
    let mut proposed = booking_entry(&format!("Slot/{}", Uuid::new_v4()));
    proposed["resource"]["status"] = json!("proposed");
    let mut two_slots = booking_entry("Slot/a");
    two_slots["resource"]["slot"] = json!([{ "reference": "Slot/a" }, { "reference": "Slot/b" }]);
    let mut no_patient = coverage_entry();
    no_patient["resource"]
        .as_object_mut()
        .unwrap()
        .remove("beneficiary");
    let mut no_bed = admission_entry(Uuid::new_v4());
    no_bed["resource"]
        .as_object_mut()
        .unwrap()
        .remove("location");
    for entry in [proposed, two_slots, no_patient, no_bed] {
        let (status, body) = post(&s, bundle("transaction", vec![entry])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(records(&s).await, before);
}

trait ZipAdd {
    fn zip_add(self, by: [i64; 4]) -> [i64; 4];
}

impl ZipAdd for [i64; 4] {
    fn zip_add(self, by: [i64; 4]) -> [i64; 4] {
        std::array::from_fn(|i| self[i] + by[i])
    }
}

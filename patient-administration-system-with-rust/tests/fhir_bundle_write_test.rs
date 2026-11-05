//! FR-202: `POST /fhir` writes a `transaction` or `batch` Bundle of Schedule and Slot. Needs PostgreSQL:
//! `cargo test -- --ignored --test-threads=1`, because it counts rows before and after a request.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::AppState;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn count(s: &AppState, sql: &str) -> i64 {
    s.db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("SELECT count(*) AS n FROM {sql}"),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<i64>("", "n")
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

trait ZipAdd {
    fn zip_add(self, by: [i64; 4]) -> [i64; 4];
}

impl ZipAdd for [i64; 4] {
    fn zip_add(self, by: [i64; 4]) -> [i64; 4] {
        std::array::from_fn(|i| self[i] + by[i])
    }
}

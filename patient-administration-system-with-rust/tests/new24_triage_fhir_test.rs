//! NEW-24: a triage category as a FHIR Observation. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::triage::TriageService;
use patient_administration_system::triage::interchange::category_of_observation;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: String) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn assessments_come_back_as_observations_oldest_first_and_round_trip() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TriageService::new(state.db.clone());
    let (enc, person) = (Uuid::new_v4(), Uuid::new_v4());
    state
        .db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO encounters (id, person_id, class, status, period_start, created_at, updated_at) \
             VALUES ($1, $2, 'emergency', 'arrived', now(), now(), now())",
            [enc.into(), person.into()],
        ))
        .await
        .unwrap();
    let now = Utc::now();
    for (cat, ago) in [("urgent", 30), ("very_urgent", 5)] {
        svc.record(
            enc,
            "mts",
            Some("3.8"),
            cat,
            None,
            "Nurse Okafor",
            now - Duration::minutes(ago),
            now,
            &UserContext::system(),
        )
        .await
        .unwrap();
    }

    let app = fhir_router(state.clone());
    let (status, body) = get(&app, format!("/fhir/Observation?encounter=Encounter/{enc}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    let cats: Vec<_> = body["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| category_of_observation(&e["resource"]))
        .collect();
    assert_eq!(cats, [Some("urgent"), Some("very_urgent")]);
    let first = &body["entry"][0]["resource"];
    assert_eq!(first["subject"]["reference"], format!("Patient/{person}"));
    assert_eq!(first["encounter"]["reference"], format!("Encounter/{enc}"));

    // The encounter is required and must exist.
    let (status, _) = get(&app, "/fhir/Observation".to_string()).await;
    assert!(status.is_client_error());
    let (status, _) = get(
        &app,
        format!("/fhir/Observation?encounter={}", Uuid::new_v4()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

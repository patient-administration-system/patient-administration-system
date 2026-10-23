//! NEW-6: reasonable adjustments as a coded record. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Utc;
use patient_administration_system::adjustments::{AdjustmentService, NewAdjustment};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::db::repositories::audit::UserContext;
use tower::ServiceExt;
use uuid::Uuid;

fn new<'a>(code: &'a str, by: &'a str) -> NewAdjustment<'a> {
    NewAdjustment {
        code,
        note: Some("prefers the front entrance"),
        review_by: None,
        recorded_by: by,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_recorded_adjustment_round_trips_through_the_api_and_fhir() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = AdjustmentService::new(state.db.clone());
    let person = Uuid::new_v4();
    let ctx = UserContext::system();
    let a = svc
        .record(person, &new("step_free", "Receptionist"), Utc::now(), &ctx)
        .await
        .unwrap();
    assert_eq!(a.category, "physical_access");

    // The API's list carries it, with its Flag.
    let live = svc.for_patient(person, false, 101).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].code, "step_free");
    assert_eq!(live[0].fhir["resourceType"], "Flag");

    // The FHIR search returns the same code, status and subject.
    let app = fhir_router(state.clone());
    let res = app
        .clone()
        .oneshot(
            Request::get(format!("/fhir/Flag?patient=Patient/{person}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["total"], 1);
    let flag = &body["entry"][0]["resource"];
    assert_eq!(flag["status"], "active");
    assert_eq!(flag["code"]["coding"][0]["code"], "step_free");
    assert_eq!(flag["subject"]["reference"], format!("Patient/{person}"));
    assert_eq!(flag["code"]["text"], "prefers the front entrance");

    // Ending it makes the Flag inactive and takes it off the live list.
    svc.end(a.id, "Receptionist", Utc::now(), &ctx)
        .await
        .unwrap();
    assert!(
        svc.for_patient(person, false, 101)
            .await
            .unwrap()
            .is_empty()
    );
    let res = app
        .oneshot(
            Request::get(format!("/fhir/Flag?patient={person}&status=inactive"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["entry"][0]["resource"]["status"], "inactive");
    assert_eq!(common::audit_count(&state.db, a.id, "record").await, 1);
    assert_eq!(common::audit_count(&state.db, a.id, "end").await, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn bad_input_is_refused_and_a_live_duplicate_conflicts() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = AdjustmentService::new(state.db.clone());
    let person = Uuid::new_v4();
    let ctx = UserContext::system();
    let now = Utc::now();

    assert!(
        svc.record(person, &new("telepathy", "Receptionist"), now, &ctx)
            .await
            .is_err(),
        "a code outside the catalogue"
    );
    assert!(
        svc.record(person, &new("braille", "  "), now, &ctx)
            .await
            .is_err(),
        "a blank name"
    );
    let long = "x".repeat(201);
    let mut too_long = new("braille", "Receptionist");
    too_long.note = Some(&long);
    assert!(svc.record(person, &too_long, now, &ctx).await.is_err());
    let mut past = new("braille", "Receptionist");
    past.review_by = Some(now.date_naive() - chrono::Days::new(1));
    assert!(svc.record(person, &past, now, &ctx).await.is_err());

    let first = svc
        .record(person, &new("braille", "Receptionist"), now, &ctx)
        .await
        .unwrap();
    assert!(
        svc.record(person, &new("braille", "Receptionist"), now, &ctx)
            .await
            .is_err(),
        "the same code twice while live"
    );
    svc.end(first.id, "Receptionist", now, &ctx).await.unwrap();
    assert!(
        svc.end(first.id, "Receptionist", now, &ctx).await.is_err(),
        "ended once only"
    );
    svc.record(person, &new("braille", "Receptionist"), now, &ctx)
        .await
        .expect("an ended adjustment may be recorded again");

    // The search needs a patient.
    let res = fhir_router(state)
        .oneshot(Request::get("/fhir/Flag").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_pictorial_pain_scale_preference_is_recorded_listed_and_not_duplicated() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = AdjustmentService::new(state.db.clone());
    let person = Uuid::new_v4();
    let ctx = UserContext::system();
    let a = svc
        .record(
            person,
            &new("pictorial_pain_scale", "Receptionist"),
            Utc::now(),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(a.category, "communication");

    let live = svc.for_patient(person, false, 101).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].code, "pictorial_pain_scale");
    assert_eq!(live[0].fhir["resourceType"], "Flag");

    // A second live row of the same code is refused, as for every other code.
    assert!(
        svc.record(
            person,
            &new("pictorial_pain_scale", "Receptionist"),
            Utc::now(),
            &ctx
        )
        .await
        .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_schedule_view_gets_live_adjustments_for_many_appointments_at_once() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = AdjustmentService::new(state.db.clone());
    let schedule = common::new_schedule(&state.db).await.id;
    let (with, without) = (Uuid::new_v4(), Uuid::new_v4());
    let ctx = UserContext::system();
    let now = Utc::now();
    svc.record(with, &new("hearing_loop", "Receptionist"), now, &ctx)
        .await
        .unwrap();
    let gone = svc
        .record(with, &new("quiet_space", "Receptionist"), now, &ctx)
        .await
        .unwrap();
    svc.end(gone.id, "Receptionist", now, &ctx).await.unwrap();
    let a = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 5).await.id,
            with,
            &ctx,
        )
        .await
        .unwrap();
    let b = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 6).await.id,
            without,
            &ctx,
        )
        .await
        .unwrap();
    let got = svc.live_for_appointments(&[a.id, b.id]).await.unwrap();
    assert_eq!(got, vec![(a.id, "hearing_loop".to_string())]);
    assert!(svc.live_for_appointments(&[]).await.unwrap().is_empty());
}

//! CAP-1: structured cancellation data. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancelledBy, ReleaseClass,
};
use patient_administration_system::scheduling::CancelDetails;
use tower::ServiceExt;
use uuid::Uuid;

async fn booked(db: &sea_orm::DatabaseConnection, in_days: i64) -> Appointment {
    let start = Utc::now() + Duration::days(in_days);
    let mut a = Appointment::new(Uuid::new_v4(), start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(db, &a).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn cancel_records_who_why_when_and_notice_and_is_an_early_release() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let a = booked(&state.db, 6).await;
    let cancelled = state
        .scheduling
        .cancel_with(
            a.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: Some(BarrierCode::CannotTravel),
                legacy_reason: None,
            },
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status, AppointmentStatus::Cancelled);
    assert_eq!(cancelled.cancelled_by, Some(CancelledBy::Patient));
    assert_eq!(cancelled.barrier_code, Some(BarrierCode::CannotTravel));
    assert_eq!(
        cancelled.notice_days,
        Some(5),
        "about six days ahead is five whole days of notice"
    );
    assert!(cancelled.cancelled_at.is_some());
    assert_eq!(cancelled.release_class, Some(ReleaseClass::EarlyRelease));

    // It survives a round trip through the database.
    let again = AppointmentRepository::find_by_id(&state.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.barrier_code, Some(BarrierCode::CannotTravel));
    assert_eq!(again.release_class, Some(ReleaseClass::EarlyRelease));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_hours_before_is_a_late_release_and_a_no_show_is_a_silent_miss() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();

    let late = booked(&state.db, 0).await;
    let late = state
        .scheduling
        .cancel_with(
            late.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: None,
                legacy_reason: None,
            },
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(late.notice_days, Some(0));
    assert_eq!(late.release_class, Some(ReleaseClass::LateRelease));

    let missed = booked(&state.db, 1).await;
    let missed = state
        .scheduling
        .mark_no_show(missed.id, &ctx)
        .await
        .unwrap();
    assert_eq!(missed.release_class, Some(ReleaseClass::SilentMiss));
    assert_eq!(
        missed.notice_days, None,
        "nobody told us, so there is no notice to record"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_barrier_can_be_added_after_a_miss_but_not_to_a_future_appointment() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();

    let missed = booked(&state.db, 1).await;
    state
        .scheduling
        .mark_no_show(missed.id, &ctx)
        .await
        .unwrap();
    let with_barrier = state
        .scheduling
        .record_barrier(missed.id, BarrierCode::Caring, &ctx)
        .await
        .unwrap();
    assert_eq!(with_barrier.barrier_code, Some(BarrierCode::Caring));
    assert_eq!(
        with_barrier.release_class,
        Some(ReleaseClass::SilentMiss),
        "still a silent miss"
    );

    let future = booked(&state.db, 3).await;
    assert!(
        state
            .scheduling
            .record_barrier(future.id, BarrierCode::Work, &ctx)
            .await
            .is_err()
    );
}

async fn post(app: axum::Router, uri: &str, body: serde_json::Value) -> StatusCode {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_cancel_endpoint_takes_codes_refuses_unknown_ones_and_still_accepts_the_old_body() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state.clone());

    let a = booked(&state.db, 5).await;
    let bad = post(
        app.clone(),
        &format!("/api/appointments/{}/cancel", a.id),
        serde_json::json!({ "barrier_code": "too_busy" }),
    )
    .await;
    assert_eq!(
        bad,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown code is refused"
    );
    let still = AppointmentRepository::find_by_id(&state.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        still.status,
        AppointmentStatus::Booked,
        "a refused request changes nothing"
    );

    let ok = post(
        app.clone(),
        &format!("/api/appointments/{}/cancel", a.id),
        serde_json::json!({ "barrier_code": "work", "cancelled_by": "patient" }),
    )
    .await;
    assert_eq!(ok, StatusCode::OK);
    let done = AppointmentRepository::find_by_id(&state.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.barrier_code, Some(BarrierCode::Work));

    // An older caller that sends only free text, or nothing, still works.
    let b = booked(&state.db, 4).await;
    assert_eq!(
        post(
            app.clone(),
            &format!("/api/appointments/{}/cancel", b.id),
            serde_json::json!({ "reason": "changed my mind" })
        )
        .await,
        StatusCode::OK
    );
}

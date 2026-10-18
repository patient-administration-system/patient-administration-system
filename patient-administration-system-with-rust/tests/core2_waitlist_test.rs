//! CORE-2: the waiting list stays in step with the appointments made from it.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::waitlist::WaitlistRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::waitlist::{Priority, WaitlistEntry, WaitlistStatus};
use tower::ServiceExt;
use uuid::Uuid;

struct W {
    state: AppState,
    schedule: Uuid,
    person: Uuid,
    entry: WaitlistEntry,
}

async fn world() -> W {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&state.db).await.id;
    let person = Uuid::new_v4();
    let entry = state
        .waitlist
        .add(
            None,
            person,
            "cardiology".into(),
            Priority::Routine,
            &UserContext::system(),
        )
        .await
        .unwrap();
    W {
        state,
        schedule,
        person,
        entry,
    }
}

async fn status(w: &W) -> WaitlistStatus {
    WaitlistRepository::find_entry_by_id(&w.state.db, w.entry.id)
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn book(w: &W, in_days: i64) -> Appointment {
    let slot = common::free_slot(&w.state.db, w.schedule, in_days).await;
    w.state
        .scheduling
        .book_for_entry(w.entry.id, slot.id, &UserContext::system())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn booking_from_an_entry_links_them_and_marks_the_entry_booked() {
    let w = world().await;
    assert_eq!(status(&w).await, WaitlistStatus::Waiting);
    let a = book(&w, 7).await;
    assert_eq!(a.from_waitlist_entry_id, Some(w.entry.id));
    assert_eq!(
        a.person_id, w.person,
        "the appointment is for the entry's patient"
    );
    assert_eq!(status(&w).await, WaitlistStatus::Booked);
    assert_eq!(
        common::audit_count(&w.state.db, w.entry.id, "status_changed").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_entry_that_is_already_booked_cannot_be_booked_again() {
    let w = world().await;
    book(&w, 7).await;
    let slot = common::free_slot(&w.state.db, w.schedule, 8).await;
    let err = w
        .state
        .scheduling
        .book_for_entry(w.entry.id, slot.id, &UserContext::system())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not waiting"), "{err}");
    let unused = patient_administration_system::db::repositories::slot::SlotRepository::find_by_id(
        &w.state.db,
        slot.id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        unused.status,
        patient_administration_system::models::schedule::SlotStatus::Free,
        "the refused booking took nothing"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_returns_the_patient_to_waiting_and_they_can_be_booked_again() {
    let w = world().await;
    let a = book(&w, 7).await;
    w.state
        .scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(
        status(&w).await,
        WaitlistStatus::Waiting,
        "back on the list, not removed"
    );
    let again = book(&w, 9).await;
    assert_eq!(again.status, AppointmentStatus::Booked);
    assert_eq!(status(&w).await, WaitlistStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_miss_returns_the_patient_to_waiting_it_never_removes_them() {
    let w = world().await;
    let a = book(&w, 1).await;
    w.state
        .scheduling
        .mark_no_show(a.id, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(status(&w).await, WaitlistStatus::Waiting);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn completing_the_appointment_marks_the_entry_treated() {
    let w = world().await;
    let a = book(&w, 1).await;
    w.state
        .scheduling
        .check_in(a.id, &UserContext::system())
        .await
        .unwrap();
    w.state
        .scheduling
        .complete(a.id, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(status(&w).await, WaitlistStatus::Treated);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_rebook_keeps_the_link_and_the_entry_stays_booked() {
    let w = world().await;
    let a = book(&w, 7).await;
    let target = common::free_slot(&w.state.db, w.schedule, 9).await;
    let (_old, new) = w
        .state
        .scheduling
        .rebook(a.id, target.id, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(
        new.from_waitlist_entry_id,
        Some(w.entry.id),
        "the new appointment carries the entry"
    );
    assert_eq!(status(&w).await, WaitlistStatus::Booked);
    // And a cancel of the new one still puts the patient back.
    w.state
        .scheduling
        .cancel(
            new.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(status(&w).await, WaitlistStatus::Waiting);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_entry_someone_removed_is_not_brought_back_by_a_later_cancel() {
    let w = world().await;
    let a = book(&w, 7).await;
    w.state
        .waitlist
        .remove(w.entry.id, &UserContext::system())
        .await
        .unwrap();
    w.state
        .scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(status(&w).await, WaitlistStatus::Removed);
    let still = AppointmentRepository::find_by_id(&w.state.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, AppointmentStatus::Cancelled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_books_from_an_entry_and_refuses_a_second_time() {
    let w = world().await;
    let app = patient_administration_system::api::rest::router(w.state.clone());
    let call = |slot: Uuid| {
        let app = app.clone();
        let uri = format!("/api/waitlist/{}/book", w.entry.id);
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "slot_id": slot }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }
    };
    let s1 = common::free_slot(&w.state.db, w.schedule, 7).await;
    let s2 = common::free_slot(&w.state.db, w.schedule, 8).await;
    assert_eq!(call(s1.id).await, StatusCode::OK);
    assert_eq!(call(s2.id).await, StatusCode::CONFLICT);
    assert_eq!(status(&w).await, WaitlistStatus::Booked);
}

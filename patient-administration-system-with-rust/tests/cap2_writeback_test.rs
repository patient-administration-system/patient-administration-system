//! CAP-2: write-back. A patient's reply changes the appointment now, exactly once.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancellationReason, ReleaseClass,
};
use patient_administration_system::models::schedule::SlotStatus;
use patient_administration_system::writeback::{ReplyAction, ReplyRequest, ReplyStatus};
use tower::ServiceExt;
use uuid::Uuid;

struct World {
    state: AppState,
    person: Uuid,
    appt: Appointment,
    slot: Uuid,
    schedule: Uuid,
}

async fn world(in_days: i64) -> World {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, schedule.id, in_days).await;
    let person = Uuid::new_v4();
    let appt = state
        .scheduling
        .book_slot(slot.id, person, &UserContext::system())
        .await
        .unwrap();
    World {
        state,
        person,
        appt,
        slot: slot.id,
        schedule: schedule.id,
    }
}

fn reply(w: &World, id: &str, action: ReplyAction) -> ReplyRequest {
    ReplyRequest {
        message_id: format!("{id}-{}", Uuid::new_v4()),
        action,
        appointment_id: w.appt.id,
        person_id: w.person,
        channel: Some("whatsapp".into()),
        barrier_code: None,
        new_slot_id: None,
        received_at: None,
    }
}

async fn slot_status(w: &World, slot: Uuid) -> SlotStatus {
    SlotRepository::find_by_id(&w.state.db, slot)
        .await
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_reply_frees_the_slot_straight_away_and_records_the_barrier() {
    let w = world(6).await;
    assert_eq!(slot_status(&w, w.slot).await, SlotStatus::Busy);
    let mut r = reply(&w, "cancel", ReplyAction::Cancel);
    r.barrier_code = Some(BarrierCode::CannotTravel);
    let out = w
        .state
        .writeback
        .apply(r, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out.status, ReplyStatus::Applied);
    assert!(!out.duplicate);
    let a = out.appointment.unwrap();
    assert_eq!(a.status, AppointmentStatus::Cancelled);
    assert_eq!(a.barrier_code, Some(BarrierCode::CannotTravel));
    assert_eq!(a.release_class, Some(ReleaseClass::EarlyRelease));
    assert_eq!(
        slot_status(&w, w.slot).await,
        SlotStatus::Free,
        "the slot is free again at once"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_same_message_applies_once_and_a_reused_id_for_another_reply_is_refused() {
    let w = world(5).await;
    let r = reply(&w, "dup", ReplyAction::Cancel);
    let first = w
        .state
        .writeback
        .apply(r.clone(), &UserContext::system())
        .await
        .unwrap();
    let second = w
        .state
        .writeback
        .apply(r.clone(), &UserContext::system())
        .await
        .unwrap();
    assert!(!first.duplicate && second.duplicate);
    assert_eq!(
        second.status,
        ReplyStatus::Applied,
        "the stored outcome is replayed"
    );
    assert_eq!(
        common::audit_count(&w.state.db, w.appt.id, "reply_cancel").await,
        1,
        "applied exactly once"
    );

    let mut other = r;
    other.action = ReplyAction::Confirm; // same message id, different reply
    let err = w
        .state
        .writeback
        .apply(other, &UserContext::system())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already used"), "{err}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_same_message_delivered_many_times_at_once_still_applies_once() {
    let w = world(5).await;
    let r = reply(&w, "race", ReplyAction::Cancel);
    let svc = w.state.writeback.clone();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (svc, r) = (svc.clone(), r.clone());
            tokio::spawn(async move { svc.apply(r, &UserContext::system()).await })
        })
        .collect();
    let mut applied_fresh = 0;
    for h in handles {
        let out = h.await.unwrap().expect("every delivery gets an answer");
        assert_eq!(out.status, ReplyStatus::Applied);
        if !out.duplicate {
            applied_fresh += 1;
        }
    }
    assert_eq!(applied_fresh, 1, "exactly one delivery did the work");
    assert_eq!(
        common::audit_count(&w.state.db, w.appt.id, "reply_cancel").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reply_from_the_wrong_patient_changes_nothing_and_reveals_nothing() {
    let w = world(5).await;
    let mut r = reply(&w, "wrong", ReplyAction::Cancel);
    r.person_id = Uuid::new_v4();
    let out = w
        .state
        .writeback
        .apply(r.clone(), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out.status, ReplyStatus::Rejected);
    assert_eq!(out.reject_reason.as_deref(), Some("person_mismatch"));
    assert!(
        out.appointment.is_none(),
        "no state of someone else's appointment is returned"
    );
    let a = AppointmentRepository::find_by_id(&w.state.db, w.appt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.status, AppointmentStatus::Booked);
    assert_eq!(slot_status(&w, w.slot).await, SlotStatus::Busy);
    // The refusal is recorded, so a retry gets the same answer, and still reveals nothing.
    let again = w
        .state
        .writeback
        .apply(r, &UserContext::system())
        .await
        .unwrap();
    assert!(again.duplicate && again.appointment.is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn replies_to_an_appointment_that_is_over_or_unknown_are_recorded_as_rejected() {
    let w = world(5).await;
    w.state
        .writeback
        .apply(
            reply(&w, "first", ReplyAction::Cancel),
            &UserContext::system(),
        )
        .await
        .unwrap();
    let late = w
        .state
        .writeback
        .apply(
            reply(&w, "late", ReplyAction::Cancel),
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(late.status, ReplyStatus::Rejected);
    assert!(late.reject_reason.unwrap().starts_with("not_cancellable"));
    let confirm = w
        .state
        .writeback
        .apply(
            reply(&w, "conf", ReplyAction::Confirm),
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert!(
        confirm
            .reject_reason
            .unwrap()
            .starts_with("not_confirmable")
    );

    let mut ghost = reply(&w, "ghost", ReplyAction::Cancel);
    ghost.appointment_id = Uuid::new_v4();
    let out = w
        .state
        .writeback
        .apply(ghost, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out.reject_reason.as_deref(), Some("unknown_appointment"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn confirming_sets_the_time_once_and_a_second_confirm_is_harmless() {
    let w = world(5).await;
    let a = w
        .state
        .writeback
        .apply(
            reply(&w, "c1", ReplyAction::Confirm),
            &UserContext::system(),
        )
        .await
        .unwrap();
    let t1 = a.appointment.unwrap().confirmed_at.expect("confirmed");
    let b = w
        .state
        .writeback
        .apply(
            reply(&w, "c2", ReplyAction::Confirm),
            &UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(b.status, ReplyStatus::Applied);
    assert_eq!(
        b.appointment.unwrap().confirmed_at,
        Some(t1),
        "the first confirmation stands"
    );
    assert_eq!(
        common::audit_count(&w.state.db, w.appt.id, "reply_confirm").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_rebook_reply_moves_the_patient_all_or_nothing() {
    let w = world(6).await;
    let target = common::free_slot(&w.state.db, w.schedule, 8).await;
    let mut r = reply(&w, "move", ReplyAction::Rebook);
    r.new_slot_id = Some(target.id);
    let out = w
        .state
        .writeback
        .apply(r, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out.status, ReplyStatus::Applied);
    let old = out.appointment.unwrap();
    let new = out.new_appointment.unwrap();
    assert_eq!(old.status, AppointmentStatus::Cancelled);
    assert_eq!(
        old.cancellation_reason,
        Some(CancellationReason::Rescheduled)
    );
    assert_eq!(new.status, AppointmentStatus::Booked);
    assert_eq!(new.slot_id, Some(target.id));
    assert_eq!(slot_status(&w, w.slot).await, SlotStatus::Free);
    assert_eq!(slot_status(&w, target.id).await, SlotStatus::Busy);

    // A slot someone else holds: refused, and the patient stays exactly where they were.
    let other = Uuid::new_v4();
    let taken = common::free_slot(&w.state.db, w.schedule, 9).await;
    w.state
        .scheduling
        .book_slot(taken.id, other, &UserContext::system())
        .await
        .unwrap();
    let w2_appt = new.id;
    let mut r2 = reply(&w, "move2", ReplyAction::Rebook);
    r2.appointment_id = w2_appt;
    r2.new_slot_id = Some(taken.id);
    let out2 = w
        .state
        .writeback
        .apply(r2, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out2.status, ReplyStatus::Rejected);
    assert_eq!(out2.reject_reason.as_deref(), Some("slot_unavailable"));
    let still = AppointmentRepository::find_by_id(&w.state.db, w2_appt)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        still.status,
        AppointmentStatus::Booked,
        "the refused move left the booking alone"
    );
    assert_eq!(slot_status(&w, target.id).await, SlotStatus::Busy);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn rebooking_into_a_clash_with_another_appointment_is_refused() {
    let w = world(6).await;
    // The patient's own second appointment overlaps the slot they ask to move to.
    let clash_slot = common::free_slot(&w.state.db, w.schedule, 8).await;
    w.state
        .scheduling
        .book_slot(clash_slot.id, w.person, &UserContext::system())
        .await
        .unwrap();
    let same_time = patient_administration_system::models::schedule::Slot::new(
        w.schedule,
        clash_slot.start_datetime,
        clash_slot.end_datetime,
    );
    let same_time = SlotRepository::create(&w.state.db, &same_time)
        .await
        .unwrap();
    let mut r = reply(&w, "clash", ReplyAction::Rebook);
    r.new_slot_id = Some(same_time.id);
    let out = w
        .state
        .writeback
        .apply(r, &UserContext::system())
        .await
        .unwrap();
    assert_eq!(out.reject_reason.as_deref(), Some("slot_unavailable"));
    assert_eq!(slot_status(&w, same_time.id).await, SlotStatus::Free);
}

async fn http(
    app: axum::Router,
    content_type: &str,
    uri: &str,
    body: String,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_api_takes_one_reply_and_a_json_or_csv_batch_with_per_row_results() {
    let w = world(5).await;
    let app = patient_administration_system::api::rest::router(w.state.clone());
    let uri = "/api/write-back/replies";

    // One reply.
    let id = format!("http-{}", Uuid::new_v4());
    let one = serde_json::json!({ "message_id": id, "action": "confirm", "appointment_id": w.appt.id, "person_id": w.person });
    let (s, v) = http(app.clone(), "application/json", uri, one.to_string()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["data"]["status"], "applied");

    // A refusal is 200 with status rejected (do not retry); malformed input is 400.
    let bad_person = serde_json::json!({ "message_id": format!("bp-{}", Uuid::new_v4()), "action": "cancel", "appointment_id": w.appt.id, "person_id": Uuid::new_v4() });
    let (s, v) = http(app.clone(), "application/json", uri, bad_person.to_string()).await;
    assert_eq!(
        (s, v["data"]["status"].as_str()),
        (StatusCode::OK, Some("rejected"))
    );
    let rebook_without_slot = serde_json::json!({ "message_id": "x", "action": "rebook", "appointment_id": w.appt.id, "person_id": w.person });
    let (s, _) = http(
        app.clone(),
        "application/json",
        uri,
        rebook_without_slot.to_string(),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Reusing a message id for a different reply is a conflict.
    let reuse = serde_json::json!({ "message_id": id, "action": "cancel", "appointment_id": w.appt.id, "person_id": w.person });
    let (s, _) = http(app.clone(), "application/json", uri, reuse.to_string()).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // JSON batch: a good row, a duplicate of it, and a malformed row; each reported on its own.
    let m = format!("batch-{}", Uuid::new_v4());
    let good = serde_json::json!({ "message_id": m, "action": "cancel", "appointment_id": w.appt.id, "person_id": w.person, "barrier_code": "work" });
    let junk = serde_json::json!({ "message_id": "", "action": "cancel", "appointment_id": w.appt.id, "person_id": w.person });
    let batch = serde_json::json!([good, good, junk]).to_string();
    let (s, v) = http(
        app.clone(),
        "application/json",
        "/api/write-back/batch",
        batch,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let rows = v["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["result"]["status"], "applied");
    assert_eq!(rows[0]["result"]["duplicate"], false);
    assert_eq!(rows[1]["result"]["duplicate"], true);
    assert!(rows[2]["error"].as_str().unwrap().contains("message_id"));

    // CSV batch on a fresh appointment.
    let w2 = world(5).await;
    let app2 = patient_administration_system::api::rest::router(w2.state.clone());
    let csv = format!(
        "message_id,action,appointment_id,person_id,channel,barrier_code,new_slot_id,received_at\ncsv-{},cancel,{},{},sms,forgot,,\n",
        Uuid::new_v4(),
        w2.appt.id,
        w2.person
    );
    let (s, v) = http(app2, "text/csv", "/api/write-back/batch", csv).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["data"][0]["result"]["status"], "applied");
    let a = AppointmentRepository::find_by_id(&w2.state.db, w2.appt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.barrier_code, Some(BarrierCode::Forgot));

    // A broken CSV is refused whole.
    let (s, _) = http(
        app.clone(),
        "text/csv",
        "/api/write-back/batch",
        "message_id,action\nm,explode\n".into(),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

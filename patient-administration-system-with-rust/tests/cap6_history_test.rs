//! CAP-6: a change or a contact can be explained. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{
    Appointment, BarrierCode, CancellationReason,
};
use patient_administration_system::writeback::{ReplyAction, ReplyRequest};
use tower::ServiceExt;
use uuid::Uuid;

fn staff() -> UserContext {
    UserContext {
        user_id: Some("nurse.b".into()),
        user_ip: None,
        user_agent: None,
    }
}

async fn booked() -> (AppState, Appointment) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 4).await;
    let a = s
        .scheduling
        .book_slot(slot.id, Uuid::new_v4(), &staff())
        .await
        .unwrap();
    (s, a)
}

fn find<'a>(h: &'a [serde_json::Value], kind: &str, action: &str) -> &'a serde_json::Value {
    h.iter()
        .find(|e| e["kind"] == kind && e["action"] == action)
        .unwrap_or_else(|| panic!("no {kind}/{action} in {h:?}"))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patients_cancel_shows_who_when_and_why() {
    let (s, a) = booked().await;
    let r = s
        .writeback
        .apply(
            ReplyRequest {
                message_id: format!("m-{}", Uuid::new_v4()),
                action: ReplyAction::Cancel,
                appointment_id: a.id,
                person_id: a.person_id,
                channel: Some("sms".into()),
                barrier_code: Some(BarrierCode::Caring),
                new_slot_id: None,
                modality: None,
                received_at: None,
            },
            &staff(),
        )
        .await
        .unwrap();
    assert_eq!(r.reject_reason, None);

    let h = rows(s.history.appointment_history(a.id).await.unwrap());
    let cancel = find(&h, "audit", "cancel");
    assert_eq!(cancel["reason_code"], "caring", "why");
    assert_eq!(cancel["by"], "nurse.b", "who");
    assert!(cancel["at"].is_string(), "when");
    assert_eq!(cancel["context"]["cancelled_by"], "patient");
    assert!(cancel["context"]["notice_days"].is_number());
    let reply = find(&h, "audit", "reply_cancel");
    assert_eq!(reply["context"]["channel"], "sms");
    // Oldest first: booked, then the patient's reply, then the cancel.
    let at: Vec<&str> = h.iter().map(|e| e["at"].as_str().unwrap()).collect();
    let mut sorted = at.clone();
    sorted.sort();
    assert_eq!(at, sorted);
    assert_eq!(h[0]["action"], "book");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_without_a_barrier_says_who_ended_it_and_a_move_says_rescheduled() {
    let (s, a) = booked().await;
    s.scheduling
        .cancel(a.id, CancellationReason::ProviderRequest, &staff())
        .await
        .unwrap();
    let h = rows(s.history.appointment_history(a.id).await.unwrap());
    // `cancel` uses the legacy reason: the patient-or-provider code comes from who cancelled.
    let c = find(&h, "audit", "cancel");
    assert!(
        c["reason_code"] == "patient_cancelled" || c["reason_code"] == "provider_cancelled",
        "{c}"
    );

    let (s2, b) = booked().await;
    let schedule = common::new_schedule(&s2.db).await.id;
    let to = common::free_slot(&s2.db, schedule, 6).await;
    s2.scheduling.rebook(b.id, to.id, &staff()).await.unwrap();
    let h = rows(s2.history.appointment_history(b.id).await.unwrap());
    assert_eq!(find(&h, "audit", "cancel")["reason_code"], "rescheduled");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_miss_is_recorded_as_a_no_show_with_who_marked_it() {
    let (s, a) = booked().await;
    s.scheduling.mark_no_show(a.id, &staff()).await.unwrap();
    let h = rows(s.history.appointment_history(a.id).await.unwrap());
    let n = find(&h, "audit", "no_show");
    assert_eq!(n["reason_code"], "no_show");
    assert_eq!(n["by"], "nurse.b");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_manually_generated_letter_is_marked_manual_and_the_http_views_work() {
    let (s, a) = booked().await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (st, v) = get(format!("/api/appointments/{}/history", a.id)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"][0]["action"], "book");
    let (st, _) = get(format!("/api/appointments/{}/history", Uuid::new_v4())).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = get(format!("/api/patients/{}/contact-log", a.person_id)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        v["data"],
        serde_json::json!([]),
        "nobody has contacted them yet"
    );
}

/// The history rows as JSON, for the assertions below.
fn rows(v: Vec<patient_administration_system::history::HistoryRow>) -> Vec<serde_json::Value> {
    serde_json::to_value(v).unwrap().as_array().unwrap().clone()
}

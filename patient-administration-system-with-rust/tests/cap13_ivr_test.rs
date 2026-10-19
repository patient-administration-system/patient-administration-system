//! CAP-13: voice reminders with keypad options. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Duration;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::reply_links;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    person: Uuid,
    schedule: Uuid,
    service: String,
    appt: Appointment,
    token: String,
}

async fn world(with_alternatives: bool, welsh: bool) -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let person = Uuid::new_v4();
    let own = common::free_slot(&s.db, schedule, 5).await;
    if with_alternatives {
        for d in [6, 7, 8] {
            common::free_slot(&s.db, schedule, d).await;
        }
    }
    let appt = s
        .scheduling
        .book_slot(own.id, person, &sys())
        .await
        .unwrap();
    if welsh {
        s.contact
            .put(
                person,
                ProfileInput {
                    preferred_language: Some("cy-001".into()),
                    ..Default::default()
                },
                &sys(),
            )
            .await
            .unwrap();
    }
    let token = reply_links::mint(&s.db, appt.id, person, appt.start_datetime)
        .await
        .unwrap();
    W {
        s,
        person,
        schedule,
        service,
        appt,
        token,
    }
}

impl W {
    async fn status(&self) -> AppointmentStatus {
        AppointmentRepository::find_by_id(&self.s.db, self.appt.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }
    async fn press(&self, state: &str, d: char) -> patient_administration_system::ivr::IvrReply {
        self.s
            .ivr
            .press(&self.token, state, d, &sys())
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_call_opens_with_one_choice_per_key_in_the_patients_language() {
    let w = world(false, false).await;
    let m = w.s.ivr.menu(&w.token).await.unwrap();
    assert_eq!((m.state, m.done), ("menu", false));
    assert_eq!(m.accepts, vec!['1', '2', '3', '9']);
    for want in [
        "Press 1 to confirm",
        "Press 2",
        "Press 3 to cancel",
        "Press 9",
    ] {
        assert!(m.say.contains(want), "{}", m.say);
    }
    assert!(!m.say.contains("http"), "no link is read out");

    let cy = world(false, true).await;
    let m = cy.s.ivr.menu(&cy.token).await.unwrap();
    assert!(
        m.say.contains("Pwyswch 1 i'w gadarnhau") && m.say.contains("Pwyswch 3 i'w ganslo"),
        "{}",
        m.say
    );
    assert_eq!(
        m.accepts,
        vec!['1', '2', '3', '9'],
        "the same keys in both languages"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn key_1_confirms_once_and_a_used_link_ends_the_call_without_changing_anything() {
    let w = world(false, false).await;
    let r = w.press("menu", '1').await;
    assert!(r.done && r.say.contains("confirmed"), "{}", r.say);
    let a = AppointmentRepository::find_by_id(&w.s.db, w.appt.id)
        .await
        .unwrap()
        .unwrap();
    assert!(a.confirmed_at.is_some());
    // The token is used: a repeat (a carrier retry, or a second call) is told so and nothing happens.
    let again = w.s.ivr.menu(&w.token).await.unwrap();
    assert!(again.done && again.accepts.is_empty());
    let again = w.press("menu", '3').await;
    assert!(again.done);
    assert_eq!(w.status().await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn cancel_asks_twice_and_keeping_changes_nothing() {
    let w = world(false, false).await;
    let sure = w.press("menu", '3').await;
    assert_eq!(
        (sure.state, sure.accepts.clone()),
        ("cancel_sure", vec!['1', '2'])
    );
    assert!(sure.say.contains("Are you sure"), "{}", sure.say);
    assert_eq!(
        w.status().await,
        AppointmentStatus::Booked,
        "the first key only asks"
    );
    // Keep it: back to the menu with a plain statement, and the appointment stands.
    let kept = w.press("cancel_sure", '2').await;
    assert!(kept.say.contains("has not been changed") && kept.state == "menu");
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    // A stray key at the question asks it again rather than cancelling.
    let stray = w.press("cancel_sure", '7').await;
    assert!(stray.say.contains("not one of the choices") && stray.say.contains("Are you sure"));
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    // Confirmed: cancelled.
    let done = w.press("cancel_sure", '1').await;
    assert!(done.done && done.say.contains("cancelled"), "{}", done.say);
    assert_eq!(w.status().await, AppointmentStatus::Cancelled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_on_an_urgent_pathway_is_held_for_a_person_whatever_key_is_pressed() {
    let w = world(false, false).await;
    w.s.follow_up
        .approve_policy(&w.service, "Dr A. Clinician", &sys())
        .await
        .unwrap();
    w.press("menu", '3').await;
    let done = w.press("cancel_sure", '1').await;
    assert!(
        done.done && done.say.contains("someone will phone you first"),
        "{}",
        done.say
    );
    assert_eq!(
        w.status().await,
        AppointmentStatus::Booked,
        "the slot is kept"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn key_2_offers_real_times_and_one_key_books_one() {
    let w = world(true, false).await;
    let t = w.press("menu", '2').await;
    assert_eq!(t.state, "times");
    assert!(
        t.say.contains("Press 1 for")
            && t.say.contains("Press 2 for")
            && t.say.contains("Press 0 to go back"),
        "{}",
        t.say
    );
    assert!(t.accepts.contains(&'1') && t.accepts.contains(&'0'));
    assert_eq!(
        w.status().await,
        AppointmentStatus::Booked,
        "listing times changes nothing"
    );
    // 0 goes back; a key that is not a time asks again.
    assert_eq!(w.press("times", '0').await.state, "menu");
    let bad = w.press("times", '9').await;
    assert!(bad.say.contains("not one of the choices") && bad.state == "times");
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    // 1 books the first.
    let done = w.press("times", '1').await;
    assert!(done.done && done.say.contains("moved to"), "{}", done.say);
    assert_eq!(
        w.status().await,
        AppointmentStatus::Cancelled,
        "the old appointment is released"
    );
    let _ = (w.person, w.schedule);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn key_2_with_nothing_to_offer_hands_over_to_a_person() {
    let w = world(false, false).await;
    let r = w.press("menu", '2').await;
    assert!(r.done && r.say.contains("contact you"), "{}", r.say);
    assert_eq!(
        w.status().await,
        AppointmentStatus::Booked,
        "the appointment stands"
    );
    let calls = w.s.ladder.open_calls(500).await.unwrap();
    assert!(calls.iter().any(
        |c| c["appointment_id"] == w.appt.id.to_string() && c["reason"] == "reschedule_request"
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_stray_key_repeats_the_menu_and_9_hears_it_again() {
    let w = world(false, false).await;
    let r = w.press("menu", '5').await;
    assert!(
        r.say.contains("not one of the choices")
            && r.say.contains("Press 1 to confirm")
            && r.state == "menu"
    );
    let again = w.press("menu", '9').await;
    assert!(again.say.starts_with("Your appointment is on") && !again.say.contains("not one of"));
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    assert!(
        w.s.ivr
            .press(&w.token, "nonsense", '1', &sys())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_work_and_a_bad_token_or_key_is_refused() {
    let w = world(false, false).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let call = |uri: &'static str, body: serde_json::Value| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
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
    let (st, v) = call(
        "/api/write-back/ivr/menu",
        serde_json::json!({ "token": w.token }),
    )
    .await;
    assert_eq!(
        (st, v["data"]["state"].as_str()),
        (StatusCode::OK, Some("menu"))
    );
    let (st, _) = call(
        "/api/write-back/ivr/menu",
        serde_json::json!({ "token": "0".repeat(64) }),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        "/api/write-back/ivr/press",
        serde_json::json!({ "token": w.token, "state": "menu", "digit": "12" }),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        "/api/write-back/ivr/press",
        serde_json::json!({ "token": w.token, "state": "menu", "digit": "1" }),
    )
    .await;
    assert_eq!(
        (st, v["data"]["done"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    let _ = Duration::zero();
}

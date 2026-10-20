//! PILOT-4: a raw text reply is mapped to the right appointment, or asked about, or left for staff.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::inbound::InboundText;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

/// A number no other test uses.
fn fresh_number() -> String {
    let n = Uuid::new_v4().as_u128() % 1_000_000_000;
    format!("+44 7{n:09}")
}

async fn patient(s: &AppState, mobile: &str, language: Option<&str>) -> Uuid {
    let p = Uuid::new_v4();
    s.contact
        .put(
            p,
            ProfileInput {
                mobile: Some(mobile.into()),
                preferred_language: language.map(Into::into),
                ..Default::default()
            },
            &UserContext::system(),
        )
        .await
        .unwrap();
    p
}

/// A booked appointment `days` ahead; `reminded` also records a sent text reminder for it.
async fn appointment(s: &AppState, person: Uuid, days: i64, reminded: bool) -> Appointment {
    let start = Utc::now() + Duration::days(days);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    let a = AppointmentRepository::create(&s.db, &a).await.unwrap();
    if reminded {
        s.db.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO generated_letters (id, template_id, person_id, appointment_id, rendered_subject, rendered_body, channel, status, sent_at, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,'Reminder','Hello','sms','sent',now(),now(),now())",
            [Uuid::new_v4().into(), Uuid::new_v4().into(), person.into(), a.id.into()],
        ))
        .await
        .unwrap();
    }
    a
}

fn text(from: &str, body: &str) -> InboundText {
    InboundText {
        message_id: format!("m-{}", Uuid::new_v4()),
        from: from.into(),
        body: body.into(),
        carrier: Some("test".into()),
        channel: None,
        received_at: None,
    }
}

async fn status_of(s: &AppState, id: Uuid) -> AppointmentStatus {
    AppointmentRepository::find_by_id(&s.db, id)
        .await
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_yes_confirms_the_one_reminded_appointment_and_a_redelivery_changes_nothing() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    let a = appointment(&s, p, 5, true).await;

    let msg = text(&num, " Yes! ");
    let out = s
        .inbound
        .handle(msg.clone(), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(
        (out.outcome.as_str(), out.intent.as_str()),
        ("applied", "confirm")
    );
    assert_eq!(out.appointment_id, Some(a.id));
    assert!(
        out.reply_text.as_deref().unwrap().contains("confirmed"),
        "{:?}",
        out.reply_text
    );
    let after = AppointmentRepository::find_by_id(&s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert!(after.confirmed_at.is_some());

    let again = s.inbound.handle(msg, &UserContext::system()).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(again.outcome, "applied");
    assert_eq!(
        again.reply_text, out.reply_text,
        "the same answer, not a second action"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_no_cancels_it_and_a_welsh_speaker_is_answered_in_welsh() {
    let s = state().await;
    let (en, cy) = (fresh_number(), fresh_number());
    let pe = patient(&s, &en, Some("en-001")).await;
    let pc = patient(&s, &cy, Some("cy-001")).await;
    let ae = appointment(&s, pe, 5, true).await;
    let ac = appointment(&s, pc, 5, true).await;

    let o = s
        .inbound
        .handle(text(&en, "Can’t come"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "applied");
    assert_eq!(status_of(&s, ae.id).await, AppointmentStatus::Cancelled);
    assert!(o.reply_text.unwrap().contains("cancelled"));

    // A Welsh reply from a Welsh-registered patient, with an accent, in Welsh.
    let o = s
        .inbound
        .handle(text(&cy, "NA"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "applied");
    assert_eq!(status_of(&s, ac.id).await, AppointmentStatus::Cancelled);
    assert!(o.reply_text.unwrap().contains("wedi'i ganslo"));

    // An English patient who replies in Welsh is understood, and answered in their own language.
    let en2 = fresh_number();
    let p2 = patient(&s, &en2, Some("en-001")).await;
    appointment(&s, p2, 5, true).await;
    let o = s
        .inbound
        .handle(text(&en2, "Cadarnhâu"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "applied");
    assert!(o.reply_text.unwrap().contains("confirmed"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn several_reminded_appointments_are_asked_about_and_the_number_completes_it() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    let first = appointment(&s, p, 3, true).await;
    let second = appointment(&s, p, 6, true).await;
    let _not_reminded = appointment(&s, p, 9, false).await;

    let asked = s
        .inbound
        .handle(text(&num, "cancel"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(asked.outcome, "asked");
    let q = asked.reply_text.unwrap();
    assert!(
        q.starts_with("Which appointment?")
            && q.contains("1 ")
            && q.contains("2 ")
            // Not ", 3 ": a date such as "23 Oct" also contains "3 ", so only a list item counts.
            && !q.contains(", 3 "),
        "{q}"
    );
    assert_eq!(
        status_of(&s, first.id).await,
        AppointmentStatus::Booked,
        "asking changes nothing"
    );

    // Out-of-range and non-number answers do nothing.
    let o = s
        .inbound
        .handle(text(&num, "7"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "unknown");
    // That answer ended the question (the latest message is no longer the question), so ask again.
    let _ = s
        .inbound
        .handle(text(&num, "cancel"), &UserContext::system())
        .await
        .unwrap();
    let done = s
        .inbound
        .handle(text(&num, "2"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(done.outcome, "applied");
    assert_eq!(done.appointment_id, Some(second.id));
    assert_eq!(status_of(&s, second.id).await, AppointmentStatus::Cancelled);
    assert_eq!(
        status_of(&s, first.id).await,
        AppointmentStatus::Booked,
        "only the chosen one"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_stale_question_is_not_answered_by_a_late_number() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    appointment(&s, p, 3, true).await;
    appointment(&s, p, 6, true).await;
    let mut ask = text(&num, "confirm");
    ask.received_at = Some(Utc::now() - Duration::minutes(45));
    assert_eq!(
        s.inbound
            .handle(ask, &UserContext::system())
            .await
            .unwrap()
            .outcome,
        "asked"
    );
    let late = s
        .inbound
        .handle(text(&num, "1"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(
        late.outcome, "unknown",
        "45 minutes is too long to guess what '1' meant"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reschedule_changes_nothing_and_waits_for_a_person_until_marked_handled() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    let a = appointment(&s, p, 5, true).await;
    let o = s
        .inbound
        .handle(text(&num, "R"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "needs_staff");
    assert_eq!(
        status_of(&s, a.id).await,
        AppointmentStatus::Booked,
        "the appointment stands until a person moves it"
    );
    assert!(o.reply_text.unwrap().contains("will contact you"));

    let list = s.inbound.staff_list(500).await.unwrap();
    let item = list
        .iter()
        .find(|i| i.appointment_id == Some(a.id))
        .expect("on the staff list");
    assert_eq!(item.body, "R");
    assert!(s.inbound.mark_handled(item.id).await.unwrap());
    assert!(!s.inbound.mark_handled(item.id).await.unwrap(), "only once");
    assert!(
        !s.inbound
            .staff_list(500)
            .await
            .unwrap()
            .iter()
            .any(|i| i.id == item.id)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_message_that_is_not_a_clear_keyword_changes_nothing() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    let a = appointment(&s, p, 5, true).await;
    for body in [
        "no problem, see you there",
        "I will be late",
        "👍",
        "cancel my other one",
        "",
    ] {
        let o = s
            .inbound
            .handle(text(&num, body), &UserContext::system())
            .await
            .unwrap();
        assert_eq!(o.outcome, "unknown", "{body:?}");
        assert!(o.reply_text.unwrap().contains("did not understand"));
    }
    assert_eq!(status_of(&s, a.id).await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_unknown_or_shared_number_is_never_acted_on_or_answered_but_stop_is_always_recorded() {
    let s = state().await;
    let stranger = fresh_number();
    let shared = fresh_number();
    let p1 = patient(&s, &shared, None).await;
    let p2 = patient(&s, &shared, None).await;
    let a1 = appointment(&s, p1, 5, true).await;
    let a2 = appointment(&s, p2, 5, true).await;

    let o = s
        .inbound
        .handle(text(&stranger, "cancel"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!((o.outcome.as_str(), o.reply_text), ("unmatched", None));
    let o = s
        .inbound
        .handle(text(&shared, "cancel"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(
        (o.outcome.as_str(), o.reply_text),
        ("ambiguous_number", None),
        "no guessing between two people"
    );
    assert_eq!(status_of(&s, a1.id).await, AppointmentStatus::Booked);
    assert_eq!(status_of(&s, a2.id).await, AppointmentStatus::Booked);

    // STOP is honoured from anyone, and answered in both languages when we do not know theirs.
    let o = s
        .inbound
        .handle(text(&stranger, "STOP"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "stop");
    let r = o.reply_text.unwrap();
    assert!(
        r.contains("no more text messages") && r.contains("Ni chewch ragor"),
        "{r}"
    );
    let o = s
        .inbound
        .handle(text(&shared, "stopio"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "stop");

    let list = s.inbound.staff_list(500).await.unwrap();
    assert!(list.iter().any(|i| i.outcome == "unmatched"));
    assert!(list.iter().any(|i| i.outcome == "ambiguous_number"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn without_a_recent_text_reminder_there_is_nothing_to_reply_to() {
    let s = state().await;
    let num = fresh_number();
    let p = patient(&s, &num, None).await;
    let a = appointment(&s, p, 5, false).await;
    let o = s
        .inbound
        .handle(text(&num, "Y"), &UserContext::system())
        .await
        .unwrap();
    assert_eq!(o.outcome, "no_open_reminder");
    assert_eq!(status_of(&s, a.id).await, AppointmentStatus::Booked);
    assert!(
        AppointmentRepository::find_by_id(&s.db, a.id)
            .await
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_none()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_message_id_cannot_be_reused_from_another_number_and_bad_input_is_refused() {
    let s = state().await;
    let (a, b) = (fresh_number(), fresh_number());
    let m = text(&a, "help");
    s.inbound
        .handle(m.clone(), &UserContext::system())
        .await
        .unwrap();
    let mut other = m.clone();
    other.from = b;
    assert!(
        s.inbound
            .handle(other, &UserContext::system())
            .await
            .is_err()
    );
    for bad in [
        InboundText {
            message_id: String::new(),
            ..text(&a, "y")
        },
        InboundText {
            message_id: "x\ny".into(),
            ..text(&a, "y")
        },
        InboundText {
            from: "not a number".into(),
            ..text(&a, "y")
        },
        InboundText {
            body: "x".repeat(1001),
            ..text(&a, "y")
        },
    ] {
        assert!(s.inbound.handle(bad, &UserContext::system()).await.is_err());
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_takes_a_text_and_the_staff_list_is_readable() {
    let s = state().await;
    let num = fresh_number();
    // The staff list shows the oldest waiting first and stops at 200, so a database that has collected
    // test texts over many runs would push this test's text off the end. Close the old ones.
    sea_orm::ConnectionTrait::execute_unprepared(
        &s.db,
        "UPDATE inbound_texts SET handled_at = now() WHERE handled_at IS NULL",
    )
    .await
    .unwrap();
    let p = patient(&s, &num, None).await;
    let a = appointment(&s, p, 5, true).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let body = serde_json::json!({ "message_id": format!("h-{}", Uuid::new_v4()), "from": num, "body": "R" });
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/write-back/texts")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(v["data"]["outcome"], "needs_staff");

    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/inbound-texts/open")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["appointment_id"] == a.id.to_string())
    );
}

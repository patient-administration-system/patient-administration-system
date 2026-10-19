//! PILOT-7: STOP, opt-outs and quiet hours. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::consent::{self, QuietHours};
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::inbound::InboundText;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

type Log = Arc<Mutex<Vec<(String, String)>>>;

struct Capture(&'static str, Log);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> patient_administration_system::Result<()> {
        self.1
            .lock()
            .unwrap()
            .push((self.0.to_string(), to.to_string()));
        Ok(())
    }
}

fn ctx() -> UserContext {
    UserContext::system()
}

fn number() -> String {
    format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000)
}

fn e164(n: &str) -> String {
    n.replace(' ', "")
}

/// A patient whose upstream record has `phone` and an email, with a booked appointment 2 hours away.
async fn world(phone: &str, email: &str) -> (AppState, Uuid, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert("telecom".into(), serde_json::json!([{ "system": "phone", "value": phone }, { "system": "email", "value": email }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture("sms", log.clone())))
        .with_sender(
            DeliveryChannel::WhatsApp,
            Arc::new(Capture("whatsapp", log.clone())),
        )
        .with_sender(
            DeliveryChannel::Email,
            Arc::new(Capture("email", log.clone())),
        );
    let start = Utc::now() + Duration::hours(2);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(&s.db, &a).await.unwrap();
    (s, person, log)
}

fn to(log: &Log, addr: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(_, t)| t == addr)
        .map(|(c, _)| c.clone())
        .collect()
}

async fn scan(
    s: &AppState,
    quiet: Option<QuietHours>,
) -> patient_administration_system::communication::reminders::ReminderReport {
    ReminderService::new(s.db.clone(), s.communication.clone())
        .with_quiet_hours(quiet)
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap()
}

fn text(from: &str, body: &str) -> InboundText {
    InboundText {
        message_id: format!("m-{}", Uuid::new_v4()),
        from: from.into(),
        body: body.into(),
        carrier: None,
        channel: None,
        received_at: None,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_who_opted_out_of_the_text_channels_is_sent_the_email_only_and_then_nothing() {
    let email = format!("{}@example.test", Uuid::new_v4());
    let phone = e164(&number());
    let (s, person, log) = world(&phone, &email).await;
    for ch in ["sms", "whatsapp"] {
        consent::record_optout(
            &s.db,
            &consent::person_subject(person),
            ch,
            "staff",
            Some("tester"),
        )
        .await
        .unwrap();
    }
    scan(&s, None).await;
    assert!(
        to(&log, &phone).is_empty(),
        "no text to someone who opted out: {:?}",
        log.lock().unwrap()
    );
    assert_eq!(
        to(&log, &email),
        vec!["email"],
        "the channel they did not opt out of still works"
    );

    // Opt out of that too, on a fresh appointment: nothing is sent and it is counted, not failed.
    let email2 = format!("{}@example.test", Uuid::new_v4());
    let phone2 = e164(&number());
    let (s2, p2, log2) = world(&phone2, &email2).await;
    for ch in ["sms", "whatsapp", "email"] {
        consent::record_optout(&s2.db, &consent::person_subject(p2), ch, "staff", None)
            .await
            .unwrap();
    }
    let r = scan(&s2, None).await;
    assert!(r.skipped_opted_out >= 1, "{r:?}");
    assert!(to(&log2, &phone2).is_empty() && to(&log2, &email2).is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn nothing_can_bypass_an_opt_out_not_even_a_direct_letter() {
    let (s, person, _log) = world(
        &e164(&number()),
        &format!("{}@example.test", Uuid::new_v4()),
    )
    .await;
    let tpl = ReminderService::new(s.db.clone(), s.communication.clone())
        .ensure_default_template()
        .await
        .unwrap();
    consent::record_optout(
        &s.db,
        &consent::person_subject(person),
        "sms",
        "staff",
        None,
    )
    .await
    .unwrap();
    let err = s
        .communication
        .generate_letter(
            tpl.id,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({ "appointment": { "when": "Tue 3 Nov 09:30", "start": "2026-11-03T09:30:00Z" } }),
            &ctx(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("opted out"), "{err}");
    // Another channel for the same person is unaffected.
    let ok = s
        .communication
        .generate_letter(
            tpl.id,
            person,
            None,
            DeliveryChannel::Email,
            serde_json::json!({ "appointment": { "when": "Tue 3 Nov 09:30", "start": "2026-11-03T09:30:00Z" } }),
            &ctx(),
        )
        .await;
    assert!(ok.is_ok(), "{ok:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn texting_stop_stops_that_number_even_when_we_cannot_tell_who_it_is() {
    let phone = e164(&number());
    let email = format!("{}@example.test", Uuid::new_v4());
    let (s, _person, log) = world(&phone, &email).await;
    // No communication profile holds this number, so the sender is "unmatched" but STOP still counts.
    let o = s
        .inbound
        .handle(text(&phone, "STOP"), &ctx())
        .await
        .unwrap();
    assert_eq!(o.outcome, "stop");
    scan(&s, None).await;
    assert!(
        to(&log, &phone).is_empty(),
        "the number said STOP: {:?}",
        log.lock().unwrap()
    );
    assert_eq!(
        to(&log, &email),
        vec!["email"],
        "STOP on SMS does not silence email"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn texting_start_after_stop_lets_messages_through_again() {
    let phone = e164(&number());
    let (s, _person, log) = world(&phone, &format!("{}@example.test", Uuid::new_v4())).await;
    s.inbound
        .handle(text(&phone, "stopio"), &ctx())
        .await
        .unwrap();
    let o = s
        .inbound
        .handle(text(&phone, "START"), &ctx())
        .await
        .unwrap();
    assert_eq!(o.outcome, "start");
    assert!(o.reply_text.unwrap().contains("again"));
    scan(&s, None).await;
    assert!(
        !to(&log, &phone).is_empty(),
        "messages flow again after START"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn stop_sent_on_any_text_channel_stops_all_text_channels_but_not_email() {
    let phone = e164(&number());
    let email = format!("{}@example.test", Uuid::new_v4());
    let (s, _person, log) = world(&phone, &email).await;
    let mut m = text(&phone, "STOP");
    m.channel = Some("whatsapp".into());
    s.inbound.handle(m, &ctx()).await.unwrap();
    scan(&s, None).await;
    assert!(
        to(&log, &phone).is_empty(),
        "neither WhatsApp nor SMS: {:?}",
        log.lock().unwrap()
    );
    assert_eq!(to(&log, &email), vec!["email"]);
    let mut bad = text(&phone, "STOP");
    bad.channel = Some("carrier-pigeon".into());
    assert!(
        s.inbound.handle(bad, &ctx()).await.is_err(),
        "an unknown channel is refused"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn quiet_hours_defer_the_reminder_without_losing_it() {
    let phone = e164(&number());
    let (s, _person, log) = world(&phone, &format!("{}@example.test", Uuid::new_v4())).await;
    // A window around the clinic's current time, so the test passes at any hour.
    let now = Utc::now();
    let local = now.with_timezone(&patient_administration_system::inbound::messages::timezone());
    let around = format!(
        "{}-{}",
        (local - Duration::hours(1)).format("%H:%M"),
        (local + Duration::hours(1)).format("%H:%M")
    );
    let r = ReminderService::new(s.db.clone(), s.communication.clone())
        .with_quiet_hours(QuietHours::parse(&around).ok())
        .run_once(now, Duration::hours(24))
        .await
        .unwrap();
    assert!(r.deferred_quiet_hours >= 1, "{r:?}");
    assert!(
        to(&log, &phone).is_empty(),
        "nothing overnight: {:?}",
        log.lock().unwrap()
    );

    // Morning: the same appointment is reminded now. Deferral lost nothing.
    let r = scan(&s, None).await;
    assert!(r.sent >= 1, "{r:?}");
    assert!(!to(&log, &phone).is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn staff_can_record_list_and_lift_an_opt_out_over_the_api() {
    let (s, person, _log) = world(
        &e164(&number()),
        &format!("{}@example.test", Uuid::new_v4()),
    )
    .await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let call = |method: &str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        let method = method.to_string();
        async move {
            let mut b = Request::builder()
                .method(method.as_str())
                .uri(uri)
                .header("x-user-id", "nurse-1");
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            let v: serde_json::Value =
                serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
                    .unwrap_or_default();
            (st, v)
        }
    };
    let base = format!("/api/patients/{person}/channel-optouts");
    assert_eq!(
        call(
            "POST",
            base.clone(),
            Some(serde_json::json!({ "channel": "pigeon" }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            "POST",
            base.clone(),
            Some(serde_json::json!({ "channel": "sms" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, v) = call("GET", base.clone(), None).await;
    let list = v["data"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0]["channel"].as_str(), list[0]["source"].as_str()),
        (Some("sms"), Some("staff"))
    );
    assert!(
        consent::is_opted_out(&s.db, person, DeliveryChannel::Sms, None)
            .await
            .unwrap()
    );
    assert_eq!(
        call("DELETE", format!("{base}/sms"), None).await.0,
        StatusCode::OK
    );
    assert!(
        !consent::is_opted_out(&s.db, person, DeliveryChannel::Sms, None)
            .await
            .unwrap()
    );
    assert_eq!(
        call("DELETE", format!("{base}/sms"), None).await.0,
        StatusCode::NOT_FOUND,
        "nothing left to lift"
    );
    assert_eq!(
        call("DELETE", format!("{base}/pigeon"), None).await.0,
        StatusCode::BAD_REQUEST
    );
}

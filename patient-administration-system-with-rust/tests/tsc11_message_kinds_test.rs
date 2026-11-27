//! TSC-11: a patient can refuse a kind of message (surveys, wording experiments) on its own, and still get
//! the messages about their appointments. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::consent;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::inbound::InboundText;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::{
    DeliveryChannel, LetterTemplate, OptOutSource, OptionalKind,
};
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

type Log = Arc<Mutex<Vec<(String, String)>>>; // (to, body)

struct Capture(Log);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        body: &str,
    ) -> patient_administration_system::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((to.to_string(), body.to_string()));
        Ok(())
    }
}

fn sys() -> UserContext {
    UserContext::system()
}

fn uniq(p: &str) -> String {
    format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..10])
}

fn phone() -> String {
    format!("+447{:09}", Uuid::new_v4().as_u128() % 1_000_000_000)
}

/// Patients with these numbers, each with a booked appointment two hours away, and SMS captured.
async fn world(phones: &[&str]) -> (AppState, Vec<(Uuid, Uuid)>, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let stub = Arc::new(StubPersonClient::new());
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub.clone())
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(log.clone())));
    let mut out = Vec::new();
    for p in phones {
        let person = Uuid::new_v4();
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
        data.insert(
            "telecom".into(),
            serde_json::json!([{ "system": "phone", "value": p }]),
        );
        stub.insert(Person { id: person, data });
        let start = Timestamp::now() + SignedDuration::from_hours(2);
        let mut a = Appointment::new(person, start, start + SignedDuration::from_mins(30));
        a.status = AppointmentStatus::Booked;
        AppointmentRepository::create(&s.db, &a).await.unwrap();
        out.push((person, a.id));
    }
    (s, out, log)
}

/// A template group in English and Welsh, by SMS, whose bodies carry `marker`.
async fn group(s: &AppState, name: &str, marker: &str) -> Uuid {
    let mut first = None;
    for locale in ["en-001", "cy-001"] {
        let mut t = LetterTemplate::new(name.into(), "Message".into(), format!("{marker} message"));
        t.locale = locale.into();
        t.channels = vec![DeliveryChannel::Sms];
        let t = LetterRepository::create_template(&s.db, &t).await.unwrap();
        first.get_or_insert(t.id);
    }
    first.unwrap()
}

async fn call(
    s: &AppState,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let app = patient_administration_system::api::rest::router(s.clone());
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let r = app.oneshot(req).await.unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn sent_to(log: &Log, to: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(t, _)| t == to)
        .map(|(_, b)| b.clone())
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_who_refuses_surveys_still_gets_reminders_and_never_a_survey() {
    let (refuser, other) = (phone(), phone());
    let (s, people, log) = world(&[&refuser, &other]).await;
    let [(person, _), (other_person, _)] = people[..] else {
        unreachable!()
    };

    // A survey group, marked as one through the API; an unknown group or kind is refused.
    let name = uniq("survey");
    let survey = group(&s, &name, "SURVEY").await;
    let (st, v) = call(
        &s,
        "PUT",
        &format!("/api/letter-templates/kinds/{name}"),
        Some(serde_json::json!({"kind": "survey"})),
    )
    .await;
    assert_eq!(
        (st, v["data"]["kind"].as_str()),
        (StatusCode::OK, Some("survey")),
        "{v}"
    );
    let (st, _) = call(
        &s,
        "PUT",
        &format!("/api/letter-templates/kinds/{}", uniq("nothing")),
        Some(serde_json::json!({"kind": "survey"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        &s,
        "PUT",
        &format!("/api/letter-templates/kinds/{name}"),
        Some(serde_json::json!({"kind": "marketing"})),
    )
    .await;
    assert!(st.is_client_error(), "{st}");

    // Staff record the refusal; the list says so.
    let (st, v) = call(
        &s,
        "POST",
        &format!("/api/patients/{person}/message-kind-refusals"),
        Some(serde_json::json!({"kind": "survey"})),
    )
    .await;
    assert_eq!(
        (st, v["data"]["refused"].as_bool()),
        (StatusCode::OK, Some(true)),
        "{v}"
    );
    let (_, v) = call(
        &s,
        "GET",
        &format!("/api/patients/{person}/message-kind-refusals"),
        None,
    )
    .await;
    assert_eq!(v["data"][0]["kind"], "survey", "{v}");
    assert_eq!(v["data"][0]["source"], "staff");

    // The survey is refused for them, on any channel, and sent to the other patient.
    let refused = s
        .communication
        .generate_letter(
            survey,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await;
    assert!(
        matches!(&refused, Err(patient_administration_system::Error::Conflict(m)) if m.contains("survey")),
        "{refused:?}"
    );
    s.communication
        .generate_letter(
            survey,
            other_person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    assert!(sent_to(&log, &other).iter().any(|b| b.contains("SURVEY")));
    assert!(sent_to(&log, &refuser).is_empty());

    // The reminder about their appointment, a service message, still goes to them.
    ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Timestamp::now(), SignedDuration::from_hours(24))
        .await
        .unwrap();
    let theirs = sent_to(&log, &refuser);
    assert!(
        !theirs.is_empty() && theirs.iter().all(|b| !b.contains("SURVEY")),
        "{theirs:?}"
    );

    // Lifted, the survey goes to them too; lifting twice is a 404; a group made a service message again
    // is never held back by a refusal.
    let (st, _) = call(
        &s,
        "DELETE",
        &format!("/api/patients/{person}/message-kind-refusals/survey"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &s,
        "DELETE",
        &format!("/api/patients/{person}/message-kind-refusals/survey"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    s.communication
        .generate_letter(
            survey,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    consent::refuse_kind(
        &s.db,
        &consent::person_subject(person),
        OptionalKind::Survey,
        OptOutSource::Staff,
        None,
    )
    .await
    .unwrap();
    let (_, v) = call(
        &s,
        "PUT",
        &format!("/api/letter-templates/kinds/{name}"),
        Some(serde_json::json!({"kind": null})),
    )
    .await;
    assert!(v["data"]["kind"].is_null(), "{v}");
    s.communication
        .generate_letter(
            survey,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
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
async fn texting_stop_surveys_refuses_surveys_only_and_start_surveys_lifts_it() {
    let number = phone();
    let (s, people, _log) = world(&[&number]).await;
    let person = people[0].0;
    let o = s
        .inbound
        .handle(text(&number, "STOP SURVEYS"), &sys())
        .await
        .unwrap();
    assert_eq!(o.outcome, "stop_kind");
    assert!(
        o.reply_text.unwrap().contains("appointments"),
        "the reply says appointment messages carry on"
    );
    assert!(
        consent::refuses_kind(&s.db, person, Some(&number), OptionalKind::Survey)
            .await
            .unwrap()
    );
    assert!(
        !consent::is_opted_out(&s.db, person, DeliveryChannel::Sms, Some(&number))
            .await
            .unwrap(),
        "not a STOP"
    );
    let o = s
        .inbound
        .handle(text(&number, "dechrau arolygon"), &sys())
        .await
        .unwrap();
    assert_eq!(o.outcome, "start_kind");
    assert!(
        !consent::refuses_kind(&s.db, person, Some(&number), OptionalKind::Survey)
            .await
            .unwrap()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_who_refuses_experiments_gets_the_usual_wording_and_is_not_assigned() {
    let (s, people, _log) = world(&[&phone(), &phone()]).await;
    let [(refuser, refuser_appt), (_, other_appt)] = people[..] else {
        unreachable!()
    };
    let (a, b, base) = (uniq("a"), uniq("b"), uniq("msg"));
    group(&s, &a, "A").await;
    group(&s, &b, "B").await;
    s.experiments
        .start(
            &base,
            vec![("A".into(), a.clone()), ("B".into(), b.clone())],
            "Dr A",
            &sys(),
        )
        .await
        .unwrap();
    consent::refuse_kind(
        &s.db,
        &consent::person_subject(refuser),
        OptionalKind::Experiment,
        OptOutSource::Staff,
        None,
    )
    .await
    .unwrap();
    let name = patient_administration_system::experiments::resolve_name(&s.db, refuser_appt, &base)
        .await
        .unwrap();
    assert_eq!(name, base);
    let n =
        s.db.count(Stmt::new(
            "SELECT count(*) AS n FROM message_experiment_assignments WHERE appointment_id = $1",
            [refuser_appt.into()],
        ))
        .await
        .unwrap();
    assert_eq!(n, 0, "not assigned, so not in the read-out");
    let other = patient_administration_system::experiments::resolve_name(&s.db, other_appt, &base)
        .await
        .unwrap();
    assert!(other == a || other == b, "{other}");
}

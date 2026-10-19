//! PILOT-5 / CAP-9: Welsh and English together. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

type Log = Arc<Mutex<Vec<(String, String)>>>;

struct Capture(Log);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        body: &str,
    ) -> patient_administration_system::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((to.to_string(), body.to_string()));
        Ok(())
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = r.status();
    (
        st,
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
            .unwrap_or_default(),
    )
}

fn variants(en: &str, cy: &str) -> serde_json::Value {
    serde_json::json!([
        { "locale": "en-001", "subject": "Hello", "body_tera": en },
        { "locale": "cy-001", "subject": "Helo", "body_tera": cy }
    ])
}

fn name() -> String {
    format!("p5-{}", Uuid::new_v4())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_pair_is_created_together_and_a_one_language_template_is_refused() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let n = name();

    // One language alone: refused, and nothing stored.
    let (st, v) = call(&app, "POST", "/api/letter-template-groups", Some(serde_json::json!({ "name": n, "variants": [{ "locale": "en-001", "subject": "Hi", "body_tera": "Hi" }] }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("cy-001"),
        "{v}"
    );
    assert!(
        LetterRepository::list_active_by_name(&s.db, &n)
            .await
            .unwrap()
            .is_empty()
    );

    // The same through the single-template endpoint.
    let single = serde_json::json!({ "name": n, "subject": "Hi", "body_tera": "Hi", "channels": ["sms"], "required_variables": [], "active": true, "locale": "en-001",
        "id": Uuid::nil(), "created_at": Utc::now(), "updated_at": Utc::now() });
    let (st, _) = call(&app, "POST", "/api/letter-templates", Some(single)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        LetterRepository::list_active_by_name(&s.db, &n)
            .await
            .unwrap()
            .is_empty()
    );

    // A bad body in one variant stores neither.
    let (st, _) = call(
        &app,
        "POST",
        "/api/letter-template-groups",
        Some(serde_json::json!({ "name": n, "variants": variants("ok", "{{ oops") })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        LetterRepository::list_active_by_name(&s.db, &n)
            .await
            .unwrap()
            .is_empty(),
        "atomic: neither was stored"
    );

    // A duplicated or malformed locale is refused.
    let dup = serde_json::json!([{ "locale": "en-001", "subject": "a", "body_tera": "a" }, { "locale": "en-001", "subject": "a", "body_tera": "a" }]);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/letter-template-groups",
            Some(serde_json::json!({ "name": n, "variants": dup }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let bad = serde_json::json!([{ "locale": "English", "subject": "a", "body_tera": "a" }, { "locale": "cy-001", "subject": "a", "body_tera": "a" }]);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/letter-template-groups",
            Some(serde_json::json!({ "name": n, "variants": bad }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    // The pair, together.
    let (st, v) = call(&app, "POST", "/api/letter-template-groups", Some(serde_json::json!({ "name": n, "channels": ["sms"], "variants": variants("Hello", "Helo") }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["variants"].as_array().unwrap().len(), 2);
    let active = LetterRepository::list_active_by_name(&s.db, &n)
        .await
        .unwrap();
    assert_eq!(
        active.iter().map(|t| t.locale.as_str()).collect::<Vec<_>>(),
        vec!["cy-001", "en-001"]
    );

    // The name is taken.
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/letter-template-groups",
            Some(serde_json::json!({ "name": n, "variants": variants("a", "b") }))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // Switching one variant off would leave the group half-active: refused.
    let mut cy = active
        .iter()
        .find(|t| t.locale == "cy-001")
        .unwrap()
        .clone();
    cy.active = false;
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/letter-templates/{}", cy.id),
        Some(serde_json::to_value(&cy).unwrap()),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "an English-only active group is not allowed"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_group_response_shows_what_each_variant_costs_as_a_text() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let (st, v) = call(
        &app,
        "POST",
        "/api/letter-template-groups",
        Some(serde_json::json!({ "name": name(), "channels": ["sms"], "variants": variants(
            "Hello {{ patient.name.given }}, your appointment is on {{ appointment.when }}.",
            "Helo {{ patient.name.given }}, mae eich apwyntiad ar {{ appointment.when }}. Cysylltwch â ni os na allwch ddod."
        ) })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let by = |loc: &str| {
        v["data"]["variants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["template"]["locale"] == loc)
            .unwrap()["sms"]
            .clone()
    };
    assert_eq!(by("en-001")["encoding"], "gsm7");
    assert_eq!(by("cy-001")["encoding"], "ucs2");
    assert_eq!(
        by("cy-001")["forced_by"],
        "â",
        "the writer is told which letter did it"
    );

    let (st, v) = call(
        &app,
        "POST",
        "/api/letter-templates/check-sms",
        Some(serde_json::json!({ "body_tera": "Mae eich ŵyr yma", "locale": "cy-001" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["encoding"], "ucs2");
    let (st, _) = call(
        &app,
        "POST",
        "/api/letter-templates/check-sms",
        Some(serde_json::json!({ "body_tera": "{{ oops" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

async fn world(language: Option<&str>) -> (AppState, Uuid, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let phone = format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ffion" }));
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": phone }]),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(log.clone())))
        .with_sender(DeliveryChannel::WhatsApp, Arc::new(Capture(log.clone())));
    if let Some(l) = language {
        s.contact
            .put(
                person,
                ProfileInput {
                    preferred_language: Some(l.into()),
                    ..Default::default()
                },
                &UserContext::system(),
            )
            .await
            .unwrap();
    }
    let start = Utc::now() + Duration::hours(2);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    AppointmentRepository::create(&s.db, &a).await.unwrap();
    (s, person, log)
}

async fn scan(s: &AppState) {
    ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_welsh_preferring_patient_gets_the_welsh_text_and_everyone_else_english() {
    let (cy, _, cy_log) = world(Some("cy-001")).await;
    scan(&cy).await;
    let sent = cy_log
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.clone())
        .collect::<Vec<_>>();
    assert!(
        sent.iter()
            .any(|b| b.starts_with("Helo Ffion") && b.contains("Atebwch IE")),
        "{sent:?}"
    );

    let (en, _, en_log) = world(Some("en-001")).await;
    scan(&en).await;
    assert!(
        en_log
            .lock()
            .unwrap()
            .iter()
            .any(|(_, b)| !b.starts_with("Helo") && !b.contains("Atebwch"))
    );

    // No preference, and a language we do not offer, both get English.
    for lang in [None, Some("ar-EG")] {
        let (s, _, log) = world(lang).await;
        scan(&s).await;
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|(_, b)| !b.starts_with("Helo")),
            "{lang:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_letter_records_which_language_was_used_and_the_time_reads_in_the_patients_language() {
    let (s, person, log) = world(Some("cy")).await;
    scan(&s).await;
    let letters = LetterRepository::list_for_person(&s.db, person, None)
        .await
        .unwrap();
    assert_eq!(letters.len(), 1);
    assert_eq!(letters[0].locale.as_deref(), Some("cy-001"));
    let body = &log.lock().unwrap()[0].1;
    // A Welsh month abbreviation, not an English one or an RFC 3339 timestamp.
    let welsh_months = [
        "Ion", "Chwef", "Maw", "Ebr", "Mai", "Meh", "Gorff", "Awst", "Medi", "Hyd", "Tach", "Rhag",
    ];
    assert!(
        welsh_months.iter().any(|m| body.contains(m))
            && !body.contains("T0")
            && !body.contains("+00:00"),
        "{body}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reminder_is_sent_once_whichever_language_it_used() {
    let (s, _, log) = world(Some("cy-001")).await;
    scan(&s).await;
    let first = log.lock().unwrap().len();
    assert!(first >= 1);
    scan(&s).await;
    assert_eq!(
        log.lock().unwrap().len(),
        first,
        "the second scan sends nothing new"
    );
}

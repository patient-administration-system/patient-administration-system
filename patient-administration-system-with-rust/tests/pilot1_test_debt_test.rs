//! PILOT-1: the requirements that were marked "not yet tested": FR-3 (the second channel is
//! reached when the first fails), FR-7 (invalid template syntax is refused) and FR-11 (the
//! appointment date-range limits). Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::reminders::{
    MAX_FAILED_ATTEMPTS, ReminderService,
};
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

type Log = Arc<Mutex<Vec<String>>>;

/// A carrier that records every attempt and either accepts or refuses.
struct Carrier {
    name: &'static str,
    works: bool,
    log: Log,
}

#[async_trait]
impl MessageSender for Carrier {
    async fn send(
        &self,
        _to: &str,
        _subject: &str,
        _body: &str,
    ) -> patient_administration_system::Result<()> {
        self.log.lock().unwrap().push(self.name.to_string());
        if self.works {
            Ok(())
        } else {
            Err(patient_administration_system::Error::upstream(
                "carrier is down",
            ))
        }
    }
}

async fn world(
    carriers: &[(&'static str, DeliveryChannel, bool)],
) -> (
    patient_administration_system::api::rest::AppState,
    Uuid,
    Appointment,
    Log,
) {
    let log: Log = Arc::default();
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert("telecom".into(), serde_json::json!([
        { "system": "phone", "value": "+15550001111" }, { "system": "email", "value": "ada@example.test" }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let mut state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    for (name, channel, works) in carriers {
        state = state.with_sender(
            *channel,
            Arc::new(Carrier {
                name,
                works: *works,
                log: log.clone(),
            }),
        );
    }
    let start = Utc::now() + Duration::hours(2);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    let a = AppointmentRepository::create(&state.db, &a).await.unwrap();
    (state, person, a, log)
}

async fn letters_by_channel(
    state: &patient_administration_system::api::rest::AppState,
    appt: Uuid,
) -> Vec<(String, String)> {
    let rows = state.db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT channel, status FROM generated_letters WHERE appointment_id = $1 ORDER BY created_at, id", [appt.into()])).await.unwrap();
    rows.iter()
        .map(|r| {
            (
                r.try_get::<String>("", "channel").unwrap(),
                r.try_get::<String>("", "status").unwrap(),
            )
        })
        .collect()
}

// --- FR-3 --------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn when_the_first_channel_fails_the_reminder_reaches_the_second_and_stops() {
    // The default template lists WhatsApp, Apple Messages, SMS, email, in that order.
    let (s, _p, a, log) = world(&[
        ("whatsapp", DeliveryChannel::WhatsApp, false), // refuses
        ("sms", DeliveryChannel::Sms, true),            // accepts
        ("email", DeliveryChannel::Email, true),        // must NOT be reached
    ])
    .await;
    let report = ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    assert!(report.sent >= 1);
    assert_eq!(
        *log.lock().unwrap(),
        vec!["whatsapp", "sms"],
        "WhatsApp tried and failed, SMS delivered, email never tried"
    );
    assert_eq!(
        letters_by_channel(&s, a.id).await,
        vec![
            ("whatsapp".into(), "failed".into()),
            ("sms".into(), "sent".into())
        ]
    );

    // The patient is now reminded: a second scan must not send again, despite the failed WhatsApp letter.
    ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(
        log.lock().unwrap().len(),
        2,
        "no further attempts once one channel delivered"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn when_every_channel_fails_the_reminder_gives_up_after_the_cap_instead_of_retrying_for_ever()
{
    let (s, _p, a, log) = world(&[
        ("whatsapp", DeliveryChannel::WhatsApp, false),
        ("sms", DeliveryChannel::Sms, false),
        ("email", DeliveryChannel::Email, false),
    ])
    .await;
    // Without the back-off, so each scan tries again at once and the cap is reached quickly.
    let svc = ReminderService::new(s.db.clone(), s.communication.clone()).without_backoff();
    for _ in 0..8 {
        svc.run_once(Utc::now(), Duration::hours(24)).await.unwrap();
    }
    let failed = letters_by_channel(&s, a.id).await;
    assert_eq!(
        failed.len() as i64,
        MAX_FAILED_ATTEMPTS,
        "stops at the cap: {failed:?}"
    );
    assert!(failed.iter().all(|(_, st)| st == "failed"));
    assert_eq!(
        log.lock().unwrap().len() as i64,
        MAX_FAILED_ATTEMPTS,
        "and the carriers are not hammered further"
    );
    let last = svc.run_once(Utc::now(), Duration::hours(24)).await.unwrap();
    assert!(last.gave_up >= 1, "the report says it gave up");
    let template = svc.ensure_default_template().await.unwrap();
    assert!(
        LetterRepository::count_failed_for_appointment(&s.db, a.id, template.id)
            .await
            .unwrap()
            >= MAX_FAILED_ATTEMPTS
    );
}

// --- FR-7 --------------------------------------------------------------------------------------

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let r = app.oneshot(req).await.unwrap();
    let st = r.status();
    (
        st,
        serde_json::from_slice(&to_bytes(r.into_body(), 256 << 20).await.unwrap())
            .unwrap_or(serde_json::Value::Null),
    )
}

fn template_json(name: &str, subject: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "id": Uuid::nil(), "name": name, "subject": subject, "body_tera": body,
        "required_variables": [], "channels": ["sms"], "active": true,
        "created_at": Utc::now(), "updated_at": Utc::now(),
    })
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn invalid_template_syntax_is_refused_on_create_and_on_update_and_nothing_is_stored() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state.clone());
    let name = format!("fr7-{}", Uuid::new_v4());

    // Create: a broken body is refused...
    let (st, v) = call(
        app.clone(),
        "POST",
        "/api/letter-templates",
        Some(template_json(&name, "Hello", "Dear {{ patient.name")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("body_tera"),
        "{v}"
    );
    // ...and so is a broken subject.
    let (st, _) = call(
        app.clone(),
        "POST",
        "/api/letter-templates",
        Some(template_json(&name, "{% if %}", "ok")),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        LetterRepository::find_active_template_by_name(&state.db, &name)
            .await
            .unwrap()
            .is_none(),
        "nothing was stored"
    );

    // A good template is accepted...
    let (st, v) = call(
        app.clone(),
        "POST",
        "/api/letter-templates",
        Some({
            // Inactive: an active template needs every required language (CAP-9, tested elsewhere).
            let mut t = template_json(
                &name,
                "Hello {{ patient.name.given }}",
                "See you at {{ appointment.start }}",
            );
            t["active"] = serde_json::json!(false);
            t
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["data"]["id"].as_str().unwrap().to_string();

    // ...and an update to broken syntax is refused and leaves the stored text alone.
    let mut bad = template_json(&name, "Hello", "{{ oops");
    bad["id"] = serde_json::json!(id);
    let (st, _) = call(
        app.clone(),
        "PUT",
        &format!("/api/letter-templates/{id}"),
        Some(bad),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let stored = LetterRepository::find_template_by_id(&state.db, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.body_tera, "See you at {{ appointment.start }}",
        "the refused edit changed nothing"
    );

    // A valid edit goes through.
    let mut good = template_json(&name, "Hello", "New wording {{ patient.name.given }}");
    good["id"] = serde_json::json!(id);
    good["active"] = serde_json::json!(false);
    let (st, _) = call(
        app,
        "PUT",
        &format!("/api/letter-templates/{id}"),
        Some(good),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}

// --- FR-11 -------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_appointment_range_must_be_forward_and_at_most_366_days() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state);
    let from = Utc::now();
    let iso = |t: chrono::DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let url = |a: chrono::DateTime<Utc>, b: chrono::DateTime<Utc>| {
        format!("/api/appointments?from={}&to={}", iso(a), iso(b))
    };

    assert_eq!(
        call(
            app.clone(),
            "GET",
            &url(from, from + Duration::days(7)),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            &url(from, from + Duration::days(366)),
            None
        )
        .await
        .0,
        StatusCode::OK,
        "366 days is the limit and is allowed"
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            &url(from, from + Duration::days(366) + Duration::seconds(1)),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "one second over is refused"
    );
    assert_eq!(
        call(app.clone(), "GET", &url(from, from), None).await.0,
        StatusCode::BAD_REQUEST,
        "empty range"
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            &url(from + Duration::days(1), from),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "backwards"
    );
    // Missing or unparseable bounds are a client error, not a server error.
    for bad in [
        "/api/appointments",
        "/api/appointments?from=2026-01-01T00:00:00Z",
        "/api/appointments?from=yesterday&to=today",
    ] {
        let st = call(app.clone(), "GET", bad, None).await.0;
        assert!(st.is_client_error(), "{bad} gave {st}");
    }
}

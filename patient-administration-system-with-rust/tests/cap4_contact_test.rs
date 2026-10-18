//! CAP-4: reliable contact details and communication needs. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

fn ctx() -> UserContext {
    UserContext::system()
}

async fn state() -> patient_administration_system::api::rest::AppState {
    common::build_state(&common::database_url().unwrap()).await
}

fn input(mobile: &str) -> ProfileInput {
    ProfileInput {
        mobile: Some(mobile.into()),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_profile_round_trips_with_a_normalised_number_and_a_fhir_fragment() {
    let s = state().await;
    let p = Uuid::new_v4();
    let saved = s
        .contact
        .put(
            p,
            ProfileInput {
                mobile: Some("+44 7700 900123".into()),
                alternate_name: Some("Sam Jones".into()),
                alternate_relationship: Some("daughter".into()),
                alternate_phone: Some("0044 7700 900999".into()),
                alternate_is_carer: true,
                alternate_is_emergency: true,
                preferred_channel: Some("whatsapp".into()),
                preferred_language: Some("cy-001".into()),
                accessibility_needs: vec!["large_print".into(), "british_sign_language".into()],
                ..Default::default()
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(saved.row.mobile_e164.as_deref(), Some("+447700900123"));
    assert_eq!(
        saved.row.alternate_phone_e164.as_deref(),
        Some("+447700900999")
    );
    assert!(saved.mobile_stale, "never verified, so stale");
    let got = s.contact.get(p).await.unwrap().unwrap();
    assert_eq!(
        got.row.accessibility_needs,
        vec!["british_sign_language", "large_print"]
    );
    assert_eq!(
        got.fhir["communication"][0]["language"]["coding"][0]["code"],
        "cy"
    );
    assert_eq!(got.fhir["contact"][0]["name"]["text"], "Sam Jones");
    assert!(s.contact.get(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn bad_input_is_refused_and_stores_nothing() {
    let s = state().await;
    let p = Uuid::new_v4();
    for bad in [
        input("07700 900123"),
        input("+44 not a number"),
        input("+12"),
        ProfileInput {
            preferred_channel: Some("fax".into()),
            ..Default::default()
        },
        ProfileInput {
            accessibility_needs: vec!["telepathy".into()],
            ..Default::default()
        },
    ] {
        assert!(s.contact.put(p, bad, &ctx()).await.is_err());
    }
    assert!(
        s.contact.get(p).await.unwrap().is_none(),
        "nothing was stored by the refused requests"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn verifying_marks_the_number_and_changing_it_clears_the_verification() {
    let s = state().await;
    let p = Uuid::new_v4();
    s.contact
        .put(p, input("+447700900123"), &ctx())
        .await
        .unwrap();
    let v = s.contact.verify_mobile(p, "staff", &ctx()).await.unwrap();
    assert!(v.row.mobile_verified_at.is_some() && !v.mobile_stale);
    assert_eq!(v.row.mobile_verification_method.as_deref(), Some("staff"));

    // Saving the same number again keeps the verification; other edits do not disturb it.
    let same = s
        .contact
        .put(
            p,
            ProfileInput {
                preferred_language: Some("cy-001".into()),
                ..input("+44 7700 900123")
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        same.row.mobile_verified_at.is_some(),
        "same number, still verified"
    );

    // A different number is unproven.
    let changed = s
        .contact
        .put(p, input("+447700900555"), &ctx())
        .await
        .unwrap();
    assert!(
        changed.row.mobile_verified_at.is_none()
            && changed.row.mobile_verification_method.is_none()
    );
    assert!(changed.mobile_stale);

    // Nothing to verify without a number, and unknown methods are refused.
    let none = Uuid::new_v4();
    s.contact
        .put(none, ProfileInput::default(), &ctx())
        .await
        .unwrap();
    assert!(
        s.contact
            .verify_mobile(none, "staff", &ctx())
            .await
            .is_err()
    );
    assert!(
        s.contact
            .verify_mobile(p, "telepathy", &ctx())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_stale_list_has_missing_unverified_and_old_numbers_but_not_fresh_ones() {
    let s = state().await;
    let (fresh, unverified, old, nomobile) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    for (p, m) in [
        (fresh, Some("+447700900101")),
        (unverified, Some("+447700900102")),
        (old, Some("+447700900103")),
        (nomobile, None),
    ] {
        s.contact
            .put(
                p,
                ProfileInput {
                    mobile: m.map(String::from),
                    ..Default::default()
                },
                &ctx(),
            )
            .await
            .unwrap();
    }
    s.contact
        .verify_mobile(fresh, "portal", &ctx())
        .await
        .unwrap();
    s.contact
        .verify_mobile(old, "portal", &ctx())
        .await
        .unwrap();
    s.db.execute(Statement::from_sql_and_values(DbBackend::Postgres,
        "UPDATE patient_communication_profiles SET mobile_verified_at = now() - interval '400 days' WHERE person_id = $1", [old.into()])).await.unwrap();
    let stale: Vec<Uuid> = s
        .contact
        .stale(365, 1000)
        .await
        .unwrap()
        .iter()
        .map(|p| p.row.person_id)
        .collect();
    assert!(stale.contains(&unverified) && stale.contains(&old) && stale.contains(&nomobile));
    assert!(!stale.contains(&fresh));
    assert!(s.contact.stale(0, 10).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_database_refuses_bad_values_even_if_the_application_is_bypassed() {
    let s = state().await;
    let bad = |col: &str, val: &str| {
        format!(
            "INSERT INTO patient_communication_profiles (person_id, {col}) VALUES ('{}', {val})",
            Uuid::new_v4()
        )
    };
    for sql in [
        bad("mobile_e164", "'07700900123'"),
        bad("mobile_e164", "'+0123456789'"),
        bad("preferred_channel", "'fax'"),
        bad("accessibility_needs", "ARRAY['telepathy']"),
        bad("mobile_verification_method", "'carrier-pigeon'"),
    ] {
        assert!(s.db.execute_unprepared(&sql).await.is_err(), "{sql}");
    }
    // A verification time without a number is meaningless and refused.
    let no_number = format!(
        "INSERT INTO patient_communication_profiles (person_id, mobile_verified_at) VALUES ('{}', now())",
        Uuid::new_v4()
    );
    assert!(s.db.execute_unprepared(&no_number).await.is_err());
}

struct Capture(&'static str, Arc<Mutex<Vec<(String, String)>>>);
#[async_trait]
impl MessageSender for Capture {
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

async fn reminder_world(
    upstream_phone: &str,
) -> (
    patient_administration_system::api::rest::AppState,
    Uuid,
    Arc<Mutex<Vec<(String, String)>>>,
    Appointment,
) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert("telecom".into(), serde_json::json!([{ "system": "phone", "value": upstream_phone }, { "system": "email", "value": "ada@example.test" }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = state()
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
    let a = AppointmentRepository::create(&s.db, &a).await.unwrap();
    (s, person, log, a)
}

fn sent_to(log: &Arc<Mutex<Vec<(String, String)>>>, number_or_addr: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(_, to)| to == number_or_addr)
        .map(|(c, _)| c.clone())
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reminder_uses_the_preferred_channel_first_and_the_verified_profile_number() {
    let (s, person, log, _a) = reminder_world("+15550000001").await;
    s.contact
        .put(
            person,
            ProfileInput {
                mobile: Some("+447700900123".into()),
                preferred_channel: Some("sms".into()),
                ..Default::default()
            },
            &ctx(),
        )
        .await
        .unwrap();
    s.contact
        .verify_mobile(person, "staff", &ctx())
        .await
        .unwrap();
    ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    // SMS goes first (the patient's choice) and succeeds, so nothing else is tried for this patient.
    assert_eq!(
        sent_to(&log, "+447700900123"),
        vec!["sms"],
        "the verified profile number, by SMS"
    );
    assert!(
        sent_to(&log, "+15550000001").is_empty(),
        "not the upstream number"
    );
    assert!(sent_to(&log, "ada@example.test").is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_unverified_profile_number_is_not_used_and_the_upstream_one_is() {
    let (s, person, log, _a) = reminder_world("+15550000002").await;
    s.contact
        .put(
            person,
            ProfileInput {
                mobile: Some("+447700900124".into()),
                preferred_channel: Some("sms".into()),
                ..Default::default()
            },
            &ctx(),
        )
        .await
        .unwrap();
    // Never verified: must not receive the message.
    ReminderService::new(s.db.clone(), s.communication.clone())
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    assert!(
        sent_to(&log, "+447700900124").is_empty(),
        "an unproven number never receives a reminder"
    );
    assert_eq!(
        sent_to(&log, "+15550000002"),
        vec!["sms"],
        "the upstream number is used instead"
    );
}

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
        serde_json::from_slice(&to_bytes(r.into_body(), 4 << 20).await.unwrap())
            .unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_api_reads_writes_verifies_and_lists_stale_profiles() {
    let s = state().await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let p = Uuid::new_v4();
    let url = format!("/api/patients/{p}/communication-profile");
    assert_eq!(
        call(app.clone(), "GET", &url, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (st, v) = call(app.clone(), "PUT", &url, Some(serde_json::json!({ "mobile": "+44 7700 900123", "preferred_language": "cy-001", "accessibility_needs": ["audio"] }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"]["mobile_e164"], "+447700900123");
    assert_eq!(v["data"]["fhir"]["resourceType"], "Patient");
    assert_eq!(
        call(
            app.clone(),
            "PUT",
            &url,
            Some(serde_json::json!({ "mobile": "nope" }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (st, v) = call(
        app.clone(),
        "POST",
        &format!("{url}/verify-mobile"),
        Some(serde_json::json!({ "method": "portal" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"]["mobile_stale"], false);
    assert_eq!(
        call(
            app.clone(),
            "POST",
            &format!("{url}/verify-mobile"),
            Some(serde_json::json!({ "method": "x" }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (st, v) = call(
        app,
        "GET",
        "/api/communication-profiles/stale?older_than_days=365&limit=5",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["data"].is_array());
}

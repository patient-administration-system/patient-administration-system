//! QUAL-37 / NFR-25: when the audit row for a write cannot be written, the write is refused and nothing is
//! kept. A trigger makes the audit insert fail for one user id only, so no other test is affected; each route
//! is called as that user and must answer with an error and leave the change absent. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::NaiveDate;
use patient_administration_system::api::rest::router;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::consent::ConsentRepository;
use patient_administration_system::db::repositories::encounter::EncounterRepository;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::Address;
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::models::communication::LetterTemplate;
use patient_administration_system::models::consent::{Consent, ConsentType};
use patient_administration_system::models::encounter::{Encounter, EncounterClass};
use patient_administration_system::models::facility::Facility;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

const FAILS: &str = "qual37-audit-fails";

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<String>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-user-id", user);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn json<T: Serialize>(v: &T) -> Option<String> {
    Some(serde_json::to_string(v).unwrap())
}

async fn count(db: &sea_orm::DatabaseConnection, sql: &str, id: Uuid) -> i64 {
    db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        sql,
        [id.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "n")
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_write_whose_audit_row_cannot_be_written_is_refused_and_leaves_no_change() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let db = state.db.clone();
    for sql in [
        "CREATE OR REPLACE FUNCTION qual37_fail() RETURNS trigger AS $$ BEGIN \
           IF NEW.user_id = 'qual37-audit-fails' THEN RAISE EXCEPTION 'audit insert refused (qual37)'; END IF; \
           RETURN NEW; END $$ LANGUAGE plpgsql",
        "DROP TRIGGER IF EXISTS qual37 ON audit_log",
        "CREATE TRIGGER qual37 BEFORE INSERT ON audit_log FOR EACH ROW EXECUTE FUNCTION qual37_fail()",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let app = router(state.clone());
    let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

    // The control: the same kind of request by an ordinary user succeeds and is audited.
    let ok_facility = Facility::new(
        "Control".into(),
        format!("C{}", Uuid::new_v4().simple()),
        Address::default(),
    );
    let (status, _) = send(
        &app,
        "POST",
        "/api/facilities",
        "someone",
        json(&ok_facility),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Create a facility: refused, and no facility row.
    let facility = Facility::new(
        "Refused".into(),
        format!("R{}", Uuid::new_v4().simple()),
        Address::default(),
    );
    let (status, _) = send(&app, "POST", "/api/facilities", FAILS, json(&facility)).await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM facilities WHERE id = $1",
            facility.id
        )
        .await,
        0
    );

    // Create a letter template: refused, and no template row.
    let template = LetterTemplate::new("Refused".into(), "s".into(), "body".into());
    let (status, _) = send(
        &app,
        "POST",
        "/api/letter-templates",
        FAILS,
        json(&template),
    )
    .await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM letter_templates WHERE id = $1",
            template.id
        )
        .await,
        0
    );

    // Create a consent: refused, and no consent row.
    let person = Uuid::new_v4();
    let consent = Consent::new(person, ConsentType::DataProcessing, date);
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/patients/{person}/consents"),
        FAILS,
        json(&consent),
    )
    .await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM consents WHERE id = $1",
            consent.id
        )
        .await,
        0
    );

    // Revoke a consent: refused, and the consent is still active.
    let live = ConsentRepository::create(
        &db,
        &Consent::new(person, ConsentType::DataProcessing, date),
    )
    .await
    .unwrap();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/consents/{}/revoke", live.id),
        FAILS,
        None,
    )
    .await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM consents WHERE id = $1 AND status = 'active'",
            live.id
        )
        .await,
        1,
        "the consent must still be active"
    );

    // Cancel an encounter: refused, and the encounter is not cancelled.
    let enc = EncounterRepository::create(&db, &Encounter::new(person, EncounterClass::Outpatient))
        .await
        .unwrap();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/encounters/{}/cancel", enc.id),
        FAILS,
        None,
    )
    .await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM encounters WHERE id = $1 AND status = 'cancelled'",
            enc.id
        )
        .await,
        0,
        "the encounter must not be cancelled"
    );

    db.execute_unprepared("DROP TRIGGER IF EXISTS qual37 ON audit_log")
        .await
        .unwrap();
}

struct Capture(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        _b: &str,
    ) -> patient_administration_system::Result<()> {
        self.0.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

/// The reminder path: nothing is sent unless the row recording the attempt was written first.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_message_is_not_sent_when_its_attempt_cannot_be_audited() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let phone = format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": phone }]),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(sent.clone())));
    let db = state.db.clone();
    let mut t = patient_administration_system::models::communication::LetterTemplate::new(
        format!("q37-{}", Uuid::new_v4()),
        "Hello".into(),
        "Your appointment.".into(),
    );
    t.channels = vec![DeliveryChannel::Sms];
    let template = LetterRepository::create_template(&db, &t).await.unwrap().id;

    for sql in [
        "CREATE OR REPLACE FUNCTION qual37_fail_attempt() RETURNS trigger AS $$ BEGIN \
           IF NEW.user_id = 'qual37-send-fails' AND NEW.action LIKE 'send\\_%\\_attempt' THEN \
             RAISE EXCEPTION 'audit insert refused (qual37)'; END IF; RETURN NEW; END $$ LANGUAGE plpgsql",
        "DROP TRIGGER IF EXISTS qual37_attempt ON audit_log",
        "CREATE TRIGGER qual37_attempt BEFORE INSERT ON audit_log FOR EACH ROW EXECUTE FUNCTION qual37_fail_attempt()",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let ctx = |who: &str| UserContext {
        user_id: Some(who.into()),
        ..UserContext::system()
    };

    // The attempt cannot be audited: the call fails and nothing was sent.
    let refused = state
        .communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &ctx("qual37-send-fails"),
        )
        .await;
    assert!(refused.is_err(), "the send must be refused");
    assert!(sent.lock().unwrap().is_empty(), "nothing may be sent");

    // The control: an ordinary user's message is sent, and its attempt is on record.
    let letter = state
        .communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &ctx("someone"),
        )
        .await
        .unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'send_sms_attempt'",
            letter.id
        )
        .await,
        1
    );

    db.execute_unprepared("DROP TRIGGER IF EXISTS qual37_attempt ON audit_log")
        .await
        .unwrap();
}

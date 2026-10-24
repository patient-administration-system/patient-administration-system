//! QUAL-37 / NFR-25: when the audit row for a write cannot be written, the write is refused and nothing is
//! kept. A trigger makes the audit insert fail for one user id only, so no other test is affected; each route
//! is called as that user and must answer with an error and leave the change absent. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::NaiveDate;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::repositories::consent::ConsentRepository;
use patient_administration_system::db::repositories::encounter::EncounterRepository;
use patient_administration_system::models::Address;
use patient_administration_system::models::communication::LetterTemplate;
use patient_administration_system::models::consent::{Consent, ConsentType};
use patient_administration_system::models::encounter::{Encounter, EncounterClass};
use patient_administration_system::models::facility::Facility;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::Serialize;
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
    let ok_facility = Facility::new("Control".into(), "CTRL".into(), Address::default());
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
    let facility = Facility::new("Refused".into(), "NOPE".into(), Address::default());
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

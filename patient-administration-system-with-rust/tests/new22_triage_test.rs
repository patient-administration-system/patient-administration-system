//! NEW-22: a triage category assigned elsewhere, recorded on an emergency encounter. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::triage::TriageService;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

async fn encounter(db: &sea_orm::DatabaseConnection, class: &str, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO encounters (id, person_id, class, status, period_start, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, now(), now(), now())",
        [id.into(), Uuid::new_v4().into(), class.into(), status.into()],
    ))
    .await
    .unwrap();
    id
}

#[allow(
    clippy::too_many_arguments,
    reason = "one field of an assessment per argument; a struct would only restate them"
)]
async fn record(
    svc: &TriageService,
    enc: Uuid,
    category: &str,
    by: &str,
    at_minutes_ago: i64,
) -> patient_administration_system::Result<patient_administration_system::triage::TriageAssessment>
{
    let now = Utc::now();
    svc.record(
        enc,
        "mts",
        Some("3.8"),
        category,
        Some("Chest pain"),
        by,
        now - Duration::minutes(at_minutes_ago),
        now,
        &UserContext::system(),
    )
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_category_and_its_assessor_are_stored_and_a_retriage_keeps_the_first() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TriageService::new(state.db.clone());
    let enc = encounter(&state.db, "emergency", "arrived").await;

    let first = record(&svc, enc, "urgent", "Nurse Okafor", 30)
        .await
        .unwrap();
    assert_eq!(first.colour, Some("yellow"));
    record(&svc, enc, "very_urgent", "Nurse Okafor", 5)
        .await
        .unwrap();

    let history = svc.history(enc).await.unwrap();
    let cats: Vec<_> = history.iter().map(|a| a.category.as_str()).collect();
    assert_eq!(
        cats,
        ["urgent", "very_urgent"],
        "oldest first; the last is current"
    );
    assert_eq!(history[0].assessed_by, "Nurse Okafor");
    assert_eq!(history[0].system_version.as_deref(), Some("3.8"));

    let audited: i64 = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'record_triage'",
            [enc.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert_eq!(audited, 2, "each assessment is audited");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn bad_input_and_the_wrong_encounter_are_refused() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TriageService::new(state.db.clone());
    let enc = encounter(&state.db, "emergency", "arrived").await;

    assert!(
        record(&svc, enc, "red", "Nurse Okafor", 1).await.is_err(),
        "colour is not a category"
    );
    assert!(
        record(&svc, enc, "urgent", "  ", 1).await.is_err(),
        "an assessor is required"
    );
    assert!(
        record(&svc, enc, "urgent", "Nurse Okafor", -10)
            .await
            .is_err(),
        "not in the future"
    );
    let bad_system = svc
        .record(
            enc,
            "ats",
            None,
            "urgent",
            None,
            "Nurse Okafor",
            Utc::now(),
            Utc::now(),
            &UserContext::system(),
        )
        .await;
    assert!(bad_system.is_err(), "an unknown system");

    let outpatient = encounter(&state.db, "outpatient", "arrived").await;
    assert!(
        record(&svc, outpatient, "urgent", "Nurse Okafor", 1)
            .await
            .is_err()
    );
    let done = encounter(&state.db, "emergency", "finished").await;
    assert!(
        record(&svc, done, "urgent", "Nurse Okafor", 1)
            .await
            .is_err()
    );
    assert!(
        record(&svc, Uuid::new_v4(), "urgent", "Nurse Okafor", 1)
            .await
            .is_err()
    );
    assert!(
        svc.history(enc).await.unwrap().is_empty(),
        "nothing was stored"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_database_refuses_an_unknown_category() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let enc = encounter(&state.db, "emergency", "arrived").await;
    let bad = state
        .db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO triage_assessments (id, encounter_id, triage_system, category, assessed_by, assessed_at) \
             VALUES ($1, $2, 'mts', 'purple', 'x', now())",
            [Uuid::new_v4().into(), enc.into()],
        ))
        .await;
    assert!(bad.is_err());
}

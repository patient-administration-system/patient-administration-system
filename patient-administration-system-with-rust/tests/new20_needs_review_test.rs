//! NEW-20: reviewing a patient's communication needs (Accessible Information Standard, step six). Needs
//! PostgreSQL: `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::needs_review::NeedsReviewService;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

async fn profile(db: &sea_orm::DatabaseConnection, needs: &[&str]) -> Uuid {
    let id = Uuid::new_v4();
    let needs: Vec<String> = needs.iter().map(|s| s.to_string()).collect();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO patient_communication_profiles (person_id, accessibility_needs) VALUES ($1, $2)",
        [id.into(), needs.into()],
    ))
    .await
    .unwrap();
    id
}

async fn is_due(svc: &NeedsReviewService, person: Uuid, months: u32) -> bool {
    svc.due(Utc::now(), months, 501)
        .await
        .unwrap()
        .iter()
        .any(|d| d.person_id == person)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn needs_are_due_for_review_until_a_named_person_reviews_them_and_again_after_the_period() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = NeedsReviewService::new(state.db.clone());
    let with_needs = profile(&state.db, &["large_print"]).await;
    let without = profile(&state.db, &[]).await;

    assert!(is_due(&svc, with_needs, 12).await, "never reviewed: due");
    assert!(
        !is_due(&svc, without, 12).await,
        "no needs recorded: nothing to review"
    );

    assert!(
        svc.record_review(with_needs, " ", Utc::now(), &UserContext::system())
            .await
            .is_err(),
        "a named person is required"
    );
    assert!(
        is_due(&svc, with_needs, 12).await,
        "a refused review changes nothing"
    );

    svc.record_review(
        with_needs,
        "Outpatient nurse",
        Utc::now(),
        &UserContext::system(),
    )
    .await
    .unwrap();
    assert!(!is_due(&svc, with_needs, 12).await, "reviewed now: not due");

    // Thirteen months later the review is out of date again.
    svc.record_review(
        with_needs,
        "Outpatient nurse",
        Utc::now() - Duration::days(400),
        &UserContext::system(),
    )
    .await
    .unwrap();
    assert!(
        is_due(&svc, with_needs, 12).await,
        "reviewed 13 months ago: due"
    );
    assert!(
        !is_due(&svc, with_needs, 24).await,
        "within a 24-month period: not due"
    );

    let audit = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'review_needs'",
            [with_needs.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        audit.try_get::<i64>("", "n").unwrap(),
        2,
        "each review is audited"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_review_needs_a_profile_and_a_sensible_period() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = NeedsReviewService::new(state.db.clone());
    assert!(
        svc.record_review(
            Uuid::new_v4(),
            "Outpatient nurse",
            Utc::now(),
            &UserContext::system()
        )
        .await
        .is_err()
    );
    assert!(svc.due(Utc::now(), 0, 10).await.is_err());
    assert!(svc.due(Utc::now(), 121, 10).await.is_err());
}

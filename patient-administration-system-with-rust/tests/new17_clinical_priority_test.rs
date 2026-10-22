//! NEW-17: a clinician's priority on a waiting-list entry, with a review-by date. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::clinical_priority::ClinicalPriorityService;
use patient_administration_system::db::repositories::audit::UserContext;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

async fn entry(
    db: &sea_orm::DatabaseConnection,
    service: &str,
    priority: &str,
    status: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO waitlist_entries (id, person_id, target_service, priority, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now(), now())",
        [id.into(), Uuid::new_v4().into(), service.into(), priority.into(), status.into()],
    ))
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_named_clinician_sets_it_and_nothing_is_reordered_by_it() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = ClinicalPriorityService::new(state.db.clone());
    let service = format!("svc-{}", Uuid::new_v4());
    let first = entry(&state.db, &service, "routine", "waiting").await;
    let second = entry(&state.db, &service, "routine", "waiting").await;
    let review = Utc::now().date_naive() + Duration::days(30);

    for (code, by) in [("P1a", " "), ("P9", "Mr Jones, consultant")] {
        assert!(
            svc.set(first, code, review, by, &UserContext::system())
                .await
                .is_err(),
            "{code} by {by:?} is refused"
        );
    }

    let order = || async {
        state
            .waitlist
            .list_by_service(&service)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect::<Vec<_>>()
    };
    let before = order().await;
    // The later entry is marked most urgent: the waiting list's own order does not change (NFR-11).
    svc.set(
        second,
        "P1a",
        review,
        "Mr Jones, consultant",
        &UserContext::system(),
    )
    .await
    .unwrap();
    svc.set(
        first,
        "P4",
        review,
        "Mr Jones, consultant",
        &UserContext::system(),
    )
    .await
    .unwrap();
    assert_eq!(
        order().await,
        before,
        "a clinical priority reorders nothing"
    );
    assert_eq!(before, vec![first, second]);

    let audit = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'set_clinical_priority'",
            [second.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.try_get::<i64>("", "n").unwrap(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_entry_past_its_review_date_is_listed_until_it_is_reviewed() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = ClinicalPriorityService::new(state.db.clone());
    let service = format!("svc-{}", Uuid::new_v4());
    let overdue = entry(&state.db, &service, "urgent", "waiting").await;
    let due_later = entry(&state.db, &service, "urgent", "waiting").await;
    let today = Utc::now().date_naive();
    svc.set(
        overdue,
        "P2",
        today - Duration::days(3),
        "Ms Patel, consultant",
        &UserContext::system(),
    )
    .await
    .unwrap();
    svc.set(
        due_later,
        "P2",
        today + Duration::days(3),
        "Ms Patel, consultant",
        &UserContext::system(),
    )
    .await
    .unwrap();

    let listed = |rows: &[patient_administration_system::clinical_priority::OverdueReview],
                  id: Uuid| { rows.iter().any(|r| r.entry_id == id) };
    let rows = svc.overdue(today, 501).await.unwrap();
    assert!(listed(&rows, overdue), "past its review date: listed");
    assert!(!listed(&rows, due_later), "not yet due: not listed");

    // Reviewed: a new review date takes it off the list.
    svc.set(
        overdue,
        "P2",
        today + Duration::days(28),
        "Ms Patel, consultant",
        &UserContext::system(),
    )
    .await
    .unwrap();
    assert!(!listed(&svc.overdue(today, 501).await.unwrap(), overdue));

    // A closed entry is not a clinician's work any more, and cannot be given a priority.
    let closed = entry(&state.db, &service, "urgent", "treated").await;
    assert!(
        svc.set(
            closed,
            "P3",
            today,
            "Ms Patel, consultant",
            &UserContext::system()
        )
        .await
        .is_err()
    );
}

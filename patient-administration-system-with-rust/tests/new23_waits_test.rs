//! NEW-23: time to first clinician contact against the triage target. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::triage::TriageService;
use patient_administration_system::triage::targets::Targets;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

async fn encounter(
    db: &sea_orm::DatabaseConnection,
    class: &str,
    status: &str,
    arrived: chrono::DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO encounters (id, person_id, class, status, period_start, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now(), now())",
        [
            id.into(),
            Uuid::new_v4().into(),
            class.into(),
            status.into(),
            arrived.into(),
        ],
    ))
    .await
    .unwrap();
    id
}

async fn triage(svc: &TriageService, enc: Uuid, category: &str, now: chrono::DateTime<Utc>) {
    svc.record(
        enc,
        "mts",
        None,
        category,
        None,
        "Nurse Okafor",
        now,
        now,
        &UserContext::system(),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn first_contact_is_recorded_once_and_the_report_and_queue_describe_it() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TriageService::new(state.db.clone());
    let ctx = UserContext::system();
    // A window of its own, far from other tests' data: a year ago, noon.
    let base = Utc::now() - Duration::days(400);
    let now = base + Duration::hours(3);
    let from = base - Duration::minutes(1);
    let to = base + Duration::hours(2);

    let within = encounter(&state.db, "emergency", "arrived", base).await;
    let past = encounter(&state.db, "emergency", "arrived", base).await;
    let waiting = encounter(
        &state.db,
        "emergency",
        "arrived",
        base + Duration::minutes(1),
    )
    .await;
    let untriaged = encounter(&state.db, "emergency", "arrived", base).await;
    let outpatient = encounter(&state.db, "ambulatory", "arrived", base).await;
    for e in [within, past, waiting] {
        triage(&svc, e, "very_urgent", base).await;
    }

    svc.record_first_contact(within, "Dr Evans", base + Duration::minutes(10), now, &ctx)
        .await
        .unwrap();
    svc.record_first_contact(past, "Dr Evans", base + Duration::minutes(11), now, &ctx)
        .await
        .unwrap();
    svc.record_first_contact(
        untriaged,
        "Dr Evans",
        base + Duration::minutes(500),
        now,
        &ctx,
    )
    .await
    .expect_err("later than now");

    // Refused: a second record, a blank name, before arrival, in the future, a non-emergency encounter.
    assert!(
        svc.record_first_contact(within, "Dr Other", base + Duration::minutes(12), now, &ctx)
            .await
            .is_err()
    );
    assert!(
        svc.record_first_contact(waiting, "  ", base + Duration::minutes(12), now, &ctx)
            .await
            .is_err()
    );
    assert!(
        svc.record_first_contact(waiting, "Dr Evans", base, now, &ctx)
            .await
            .is_err()
    );
    assert!(
        svc.record_first_contact(
            outpatient,
            "Dr Evans",
            base + Duration::minutes(5),
            now,
            &ctx
        )
        .await
        .is_err()
    );
    let stored: Option<String> = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT first_contact_by FROM encounters WHERE id = $1",
            [within.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "first_contact_by")
        .unwrap();
    assert_eq!(
        stored.as_deref(),
        Some("Dr Evans"),
        "the first record stands"
    );

    let report = svc
        .waits_report(from, to, &Targets::default(), now)
        .await
        .unwrap();
    assert_eq!(report.attendances, 4);
    assert_eq!(report.excluded_without_category, 1);
    let vu = &report.by_category[1];
    assert_eq!(
        (vu.seen_within, vu.seen_past, vu.waiting_past),
        (1, 1, 1),
        "10 minutes is within, 11 is past, and the one still waiting at 179 minutes is past"
    );
    assert_eq!(report.past_target.len(), 2);

    // A configured target replaces the default.
    let lenient = Targets::from_json(r#"{"very_urgent": 15}"#).unwrap();
    let report = svc.waits_report(from, to, &lenient, now).await.unwrap();
    assert_eq!(report.by_category[1].seen_past, 0);

    // The queue holds those not yet seen: the waiting one, then the untriaged one last.
    let queue = svc.queue(&Targets::default(), now).await.unwrap();
    let mine: Vec<_> = queue
        .iter()
        .filter(|q| [waiting, untriaged].contains(&q.encounter_id))
        .collect();
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].encounter_id, waiting);
    assert_eq!(mine[1].category, None);

    // Windows that are backwards or too long are refused.
    assert!(
        svc.waits_report(to, from, &Targets::default(), now)
            .await
            .is_err()
    );
    assert!(
        svc.waits_report(from, from + Duration::days(40), &Targets::default(), now)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_database_refuses_a_contact_time_without_a_name() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let enc = encounter(&state.db, "emergency", "arrived", Utc::now()).await;
    let r = state
        .db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE encounters SET first_contact_at = now() WHERE id = $1",
            [enc.into()],
        ))
        .await;
    assert!(r.is_err());
}

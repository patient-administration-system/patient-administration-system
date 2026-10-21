//! Shared test harness for integration tests.
//!
//! Tests that need PostgreSQL are `#[ignore]`d, so plain `cargo test` needs no database and
//! reports them as ignored. Run them with `DATABASE_URL=… cargo test -- --ignored`; if
//! `DATABASE_URL` is missing they fail loudly.

use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;

/// The database tests are `#[ignore]`d, so reaching this means someone asked for them
/// (`cargo test -- --ignored`). A missing database is then a failure, never a silent pass.
pub fn database_url() -> Option<String> {
    Some(
        std::env::var("DATABASE_URL").expect(
            "DATABASE_URL is required: these tests need PostgreSQL (see podman/compose.yml)",
        ),
    )
}

#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn build_state(url: &str) -> AppState {
    let db = connect(url).await.expect("connect");
    let publisher = Arc::new(InMemoryEventPublisher::new());
    AppState::new(db, publisher)
}

/// A fresh schedule, so a test's slots are its own.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn new_schedule(
    db: &sea_orm::DatabaseConnection,
) -> patient_administration_system::models::schedule::Schedule {
    use patient_administration_system::db::repositories::schedule::ScheduleRepository;
    use patient_administration_system::models::schedule::{Schedule, ScheduleOwner};
    let s = Schedule::new(
        ScheduleOwner::Practitioner(uuid::Uuid::new_v4()),
        "test-clinic".into(),
    );
    ScheduleRepository::create(db, &s)
        .await
        .expect("create schedule")
}

/// A free slot starting `in_days` days from now and lasting 30 minutes.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn free_slot(
    db: &sea_orm::DatabaseConnection,
    schedule_id: uuid::Uuid,
    in_days: i64,
) -> patient_administration_system::models::schedule::Slot {
    use patient_administration_system::db::repositories::slot::SlotRepository;
    use patient_administration_system::models::schedule::Slot;
    let start = chrono::Utc::now() + chrono::Duration::days(in_days);
    let slot = Slot::new(schedule_id, start, start + chrono::Duration::minutes(30));
    SlotRepository::create(db, &slot)
        .await
        .expect("create slot")
}

/// How many audit rows exist for an entity and action.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn audit_count(
    db: &sea_orm::DatabaseConnection,
    entity_id: uuid::Uuid,
    action: &str,
) -> i64 {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = $2",
            [entity_id.into(), action.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "n").unwrap()
}

/// Like [`new_schedule`], for a named service, so a test's candidates are its own.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn new_schedule_for(
    db: &sea_orm::DatabaseConnection,
    service: &str,
) -> patient_administration_system::models::schedule::Schedule {
    use patient_administration_system::db::repositories::schedule::ScheduleRepository;
    use patient_administration_system::models::schedule::{Schedule, ScheduleOwner};
    let s = Schedule::new(
        ScheduleOwner::Practitioner(uuid::Uuid::new_v4()),
        service.into(),
    );
    ScheduleRepository::create(db, &s)
        .await
        .expect("create schedule")
}

/// A free slot starting `hours` hours from now (use for "cancelled a few hours before").
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn free_slot_in_hours(
    db: &sea_orm::DatabaseConnection,
    schedule_id: uuid::Uuid,
    hours: i64,
) -> patient_administration_system::models::schedule::Slot {
    use patient_administration_system::db::repositories::slot::SlotRepository;
    use patient_administration_system::models::schedule::Slot;
    let start = chrono::Utc::now() + chrono::Duration::hours(hours);
    let slot = Slot::new(schedule_id, start, start + chrono::Duration::minutes(30));
    SlotRepository::create(db, &slot)
        .await
        .expect("create slot")
}

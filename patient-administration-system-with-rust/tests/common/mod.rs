//! Shared test harness for integration tests.
//!
//! Tests that need PostgreSQL are `#[ignore]`d, so plain `cargo test` needs no database and
//! reports them as ignored. Run them with `DATABASE_URL=… cargo test -- --ignored`; if
//! `DATABASE_URL` is missing they fail loudly.

use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
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
    db: &patient_administration_system::db::sql::Pool,
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
    db: &patient_administration_system::db::sql::Pool,
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

/// Is a person asked to ring about this appointment for this reason? Asked of the table: `open_calls` shows the
/// first 500 open tasks, so it loses a new one on a database that has seen many runs.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn call_is_open(
    db: &patient_administration_system::db::sql::Pool,
    appointment: uuid::Uuid,
    reason: &str,
) -> bool {
    db.count(Stmt::new(
        "SELECT count(*) AS n FROM staff_call_tasks WHERE appointment_id = $1 AND reason = $2 AND handled_at IS NULL",
        [appointment.into(), reason.to_string().into()],
    ))
    .await
    .unwrap()
        > 0
}

/// How many audit rows exist for an entity and action.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn audit_count(
    db: &patient_administration_system::db::sql::Pool,
    entity_id: uuid::Uuid,
    action: &str,
) -> i64 {
    let row = db
        .one(Stmt::new(
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = $2",
            [entity_id.into(), action.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.col("n").unwrap()
}

/// Like [`new_schedule`], for a named service, so a test's candidates are its own.
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn new_schedule_for(
    db: &patient_administration_system::db::sql::Pool,
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
    db: &patient_administration_system::db::sql::Pool,
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

/// Every row of a service's list, read 500 at a time, so that a test finds its own row however many earlier
/// runs have left in a long-used database (QUAL-50).
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn every_page<T, F, Fut>(mut read: F) -> Vec<T>
where
    F: FnMut(patient_administration_system::paging::Rows) -> Fut,
    Fut: std::future::Future<Output = patient_administration_system::Result<Vec<T>>>,
{
    const PAGE: i64 = 500;
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let page = read(patient_administration_system::paging::Rows::window(
            PAGE, offset,
        ))
        .await
        .unwrap();
        let n = page.len();
        out.extend(page);
        if i64::try_from(n).unwrap() < PAGE {
            return out;
        }
        offset += PAGE;
        assert!(offset < 1_000_000, "the list never ends");
    }
}

/// Every row of an API list, following `meta.next` from `first`, as one envelope `{"success": true, "data": [...]}`; or the first
/// answer that is not a `200`, as it came (QUAL-50).
#[allow(
    dead_code,
    reason = "each test binary uses only some of the shared helpers"
)]
pub async fn every_api_page<F, Fut>(
    mut get: F,
    first: &str,
) -> (axum::http::StatusCode, serde_json::Value)
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = (axum::http::StatusCode, serde_json::Value)>,
{
    let mut rows = Vec::new();
    let mut uri = first.to_string();
    for _ in 0..10_000 {
        let (status, v) = get(uri.clone()).await;
        if status != axum::http::StatusCode::OK {
            return (status, v);
        }
        rows.extend(v["data"].as_array().cloned().unwrap_or_default());
        match v["meta"]["next"].as_str() {
            Some(next) => uri = next.to_string(),
            None => return (status, serde_json::json!({ "success": true, "data": rows })),
        }
    }
    panic!("the list never ends: {first}");
}

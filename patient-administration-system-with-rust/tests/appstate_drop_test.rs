//! Dropping the application state frees its services, and with them the database pool.
//!
//! The scheduling service once held the backfill service strongly while the backfill held the standby service,
//! which held the scheduling service: a reference cycle that kept the pool and its connections alive for the
//! life of the process, one leaked connection per test in a single-process run.

use patient_administration_system::api::rest::AppState;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;

#[tokio::test]
async fn dropping_the_state_frees_the_services() {
    let pool = sqlx::PgPool::connect_lazy("postgres://u:p@localhost:1/none").unwrap();
    let state = AppState::new(pool, Arc::new(InMemoryEventPublisher::new()));
    let scheduling = Arc::downgrade(&state.scheduling);
    let backfill = Arc::downgrade(&state.backfill);
    drop(state);
    assert!(scheduling.upgrade().is_none(), "scheduling leaked");
    assert!(backfill.upgrade().is_none(), "backfill leaked");
}

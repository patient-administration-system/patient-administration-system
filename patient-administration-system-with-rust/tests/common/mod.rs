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

#[allow(dead_code)]
pub async fn build_state(url: &str) -> AppState {
    let db = connect(url).await.expect("connect");
    let publisher = Arc::new(InMemoryEventPublisher::new());
    AppState::new(db, publisher)
}

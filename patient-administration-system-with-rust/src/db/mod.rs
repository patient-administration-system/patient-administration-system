//! db
pub mod entities;
pub mod repositories;
pub mod row;
pub mod sql;

use std::str::FromStr;
use std::time::Duration;

use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::db::sql::Pool;
use crate::{Error, Result};

/// Connect to the configured PostgreSQL database and return a `Pool`.
///
/// The pool holds at most `PAS_DB_MAX_CONNECTIONS` (default 10). It is the ceiling on work in the database at
/// once: the load test found that bookings stop getting faster with more callers once every connection is
/// busy (PERF-1, `operations/load-testing.md`). Each replica has its own pool, so replicas times this must
/// stay under the database's `max_connections`.
pub async fn connect(url: &str) -> Result<Pool> {
    let options = PgConnectOptions::from_str(url)
        .map_err(Error::Database)?
        .disable_statement_logging();
    let max = crate::config::env::count::<u32>("PAS_DB_MAX_CONNECTIONS")?;
    PgPoolOptions::new()
        .max_connections(max)
        .min_connections(max.min(2))
        // A request waits this long for a connection, so a database outage becomes a quick 503, not a hang.
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(60))
        .connect_with(options)
        .await
        .map_err(Error::Database)
}

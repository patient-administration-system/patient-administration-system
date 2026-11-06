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
pub async fn connect(url: &str) -> Result<Pool> {
    let options = PgConnectOptions::from_str(url)
        .map_err(Error::Database)?
        .disable_statement_logging();
    PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        // A request waits this long for a connection, so a database outage becomes a quick 503, not a hang.
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(60))
        .connect_with(options)
        .await
        .map_err(Error::Database)
}

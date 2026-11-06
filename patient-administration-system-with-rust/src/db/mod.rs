//! db
pub mod entities;
pub mod repositories;
pub mod row;
pub mod sql;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;

use crate::{Error, Result};

/// Connect to the configured PostgreSQL database and return a `DatabaseConnection`.
pub async fn connect(url: &str) -> Result<DatabaseConnection> {
    let mut opt = ConnectOptions::new(url.to_string());
    opt.max_connections(10)
        .min_connections(2)
        .connect_timeout(Duration::from_secs(8))
        // A request waits this long for a connection, so a database outage becomes a quick 503, not a hang.
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(60))
        .sqlx_logging(false);
    Database::connect(opt).await.map_err(Error::Database)
}

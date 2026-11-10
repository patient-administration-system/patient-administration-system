//! The PAS schema: the files in `migrations/sql`, applied in order by sqlx and embedded in the binary.
//!
//! A migration is `<version>_<name>.up.sql` with its `.down.sql`, or a single `<version>_<name>.sql` when it is
//! forward-only (the first six: undoing them would lose identity data).
//!
//! # History
//!
//! These began as SeaORM migrations. Each database's record of them was carried into `_sqlx_migrations` (SQLX-1),
//! and the carrying over is gone with SeaORM: migration 61 drops the old record, `seaql_migrations`.
#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

pub use axa_sqlx::MigrationStatus;

/// Every migration, embedded at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!("./sql");

/// Apply every migration still pending.
///
/// # Errors
///
/// Fails when a migration fails, or an applied one no longer matches its file.
pub async fn up(pool: &PgPool) -> Result<(), MigrateError> {
    axa_sqlx::migrate(pool, &MIGRATOR).await
}

/// Apply migrations up to and including `version`, and no further. For tests of the schema at a past point.
///
/// # Errors
///
/// As [`up`].
pub async fn up_to(pool: &PgPool, version: i64) -> Result<(), MigrateError> {
    let mut partial = Migrator {
        migrations: MIGRATOR.migrations.clone(),
        ignore_missing: true,
        ..Migrator::DEFAULT
    };
    partial.migrations.to_mut().retain(|m| m.version <= version);
    partial.run(pool).await
}

/// Revert the newest `steps` migrations. A forward-only migration has no down file, so it stays applied.
///
/// # Errors
///
/// Fails on a database error or a failed down migration.
pub async fn down(pool: &PgPool, steps: u32) -> Result<(), MigrateError> {
    axa_sqlx::revert(pool, &MIGRATOR, steps).await
}

/// Every migration, oldest first, with whether it has run.
///
/// # Errors
///
/// Fails on a database error.
pub async fn status(pool: &PgPool) -> Result<Vec<MigrationStatus>, MigrateError> {
    Ok(axa_sqlx::status(pool, &MIGRATOR).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migrations_are_all_there_and_the_first_six_are_forward_only() {
        let ups: Vec<_> = MIGRATOR
            .iter()
            .filter(|m| !m.migration_type.is_down_migration())
            .collect();
        assert_eq!(ups.len(), 61);
        let downs = MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_down_migration())
            .count();
        assert_eq!(downs, 55);
        assert!(ups[..6].iter().all(|m| !m.migration_type.is_reversible()));
        assert!(ups[6..].iter().all(|m| m.migration_type.is_reversible()));
    }
}

//! The PAS schema: the files in `migrations/sql`, applied in order by sqlx and embedded in the binary.
//!
//! A migration is `<version>_<name>.up.sql` with its `.down.sql`, or a single `<version>_<name>.sql` when it is
//! forward-only (the first six: undoing them would lose identity data).
//!
//! # History
//!
//! These began as SeaORM migrations, recorded in `seaql_migrations`. [`up`] carries that record into sqlx's
//! `_sqlx_migrations` once, so a database that already has the schema is never migrated twice (see [`adopt`]).
//! `seaql_migrations` is left in place as the old record, and nothing reads it after that.
#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashSet;

use sqlx::PgPool;
use sqlx::migrate::{Migrate, MigrateError, Migrator};

pub use axa_sqlx::MigrationStatus;

/// Every migration, embedded at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!("./sql");

/// The version of a SeaORM migration name: `m20260520_000001_init` is `20260520000001`.
///
/// ```
/// assert_eq!(migration::version_of("m20260520_000001_init"), Some(20260520000001));
/// assert_eq!(migration::version_of("init"), None);
/// ```
pub fn version_of(seaorm_name: &str) -> Option<i64> {
    let (date, rest) = seaorm_name.strip_prefix('m')?.split_once('_')?;
    let (sequence, _) = rest.split_once('_')?;
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(date) || !all_digits(sequence) {
        return None;
    }
    format!("{date}{sequence}").parse().ok()
}

fn refused(message: String) -> MigrateError {
    MigrateError::Source(message.into())
}

/// Carry the record in `seaql_migrations` into `_sqlx_migrations`, in one transaction, and return how many
/// migrations were carried over. This runs no migration.
///
/// It does nothing when there is no `seaql_migrations`, or when `_sqlx_migrations` already has rows: the record has
/// been carried over, and repeating it would mark a migration as applied after someone had reverted it.
///
/// # Errors
///
/// Refuses a database that records a migration this build does not have (it is newer than this binary), or one
/// whose record has a gap, and changes nothing. Fails on a database error.
pub async fn adopt(pool: &PgPool) -> Result<u64, MigrateError> {
    let mut txn = pool.begin().await?;
    let found: Option<String> = sqlx::query_scalar("SELECT to_regclass('seaql_migrations')::text")
        .fetch_one(&mut *txn)
        .await?;
    if found.is_none() {
        return Ok(0);
    }
    txn.ensure_migrations_table(&MIGRATOR.table_name).await?;
    // Two services starting together take turns here.
    sqlx::query("LOCK TABLE _sqlx_migrations IN EXCLUSIVE MODE")
        .execute(&mut *txn)
        .await?;
    let carried: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&mut *txn)
        .await?;
    if carried > 0 {
        return Ok(0);
    }

    let names: Vec<String> =
        sqlx::query_scalar("SELECT version FROM seaql_migrations ORDER BY version")
            .fetch_all(&mut *txn)
            .await?;
    let recorded = names
        .iter()
        .map(|name| {
            version_of(name).ok_or_else(|| refused(format!("seaql_migrations holds {name}")))
        })
        .collect::<Result<HashSet<i64>, _>>()?;
    let known: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .collect();
    let known_versions: HashSet<i64> = known.iter().map(|m| m.version).collect();
    if let Some(unknown) = recorded.difference(&known_versions).min() {
        return Err(refused(format!(
            "the database records migration {unknown}, which this build does not have: it is newer than this binary"
        )));
    }
    let done = known.iter().take_while(|m| recorded.contains(&m.version));
    let done: Vec<_> = done.collect();
    if done.len() != recorded.len() {
        return Err(refused(
            "seaql_migrations has a gap: an earlier migration is missing from its record"
                .to_string(),
        ));
    }

    for migration in &done {
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES ($1, $2, TRUE, $3, 0)",
        )
        .bind(migration.version)
        .bind(&*migration.description)
        .bind(&*migration.checksum)
        .execute(&mut *txn)
        .await?;
    }
    txn.commit().await?;
    Ok(done.len() as u64)
}

/// Carry over any SeaORM record, then apply every migration still pending.
///
/// # Errors
///
/// See [`adopt`]. Fails when a migration fails, or an applied one no longer matches its file.
pub async fn up(pool: &PgPool) -> Result<(), MigrateError> {
    adopt(pool).await?;
    axa_sqlx::migrate(pool, &MIGRATOR).await
}

/// Apply migrations up to and including `version`, and no further. For tests of the schema at a past point.
///
/// # Errors
///
/// As [`up`].
pub async fn up_to(pool: &PgPool, version: i64) -> Result<(), MigrateError> {
    adopt(pool).await?;
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
    adopt(pool).await?;
    axa_sqlx::revert(pool, &MIGRATOR, steps).await
}

/// Every migration, oldest first, with whether it has run.
///
/// # Errors
///
/// As [`adopt`].
pub async fn status(pool: &PgPool) -> Result<Vec<MigrationStatus>, MigrateError> {
    adopt(pool).await?;
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
        assert_eq!(ups.len(), 60);
        let downs = MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_down_migration())
            .count();
        assert_eq!(downs, 54);
        assert!(ups[..6].iter().all(|m| !m.migration_type.is_reversible()));
        assert!(ups[6..].iter().all(|m| m.migration_type.is_reversible()));
    }

    #[test]
    fn a_name_that_is_not_a_seaorm_migration_has_no_version() {
        assert_eq!(version_of("m2026_x"), None);
        assert_eq!(version_of("mabc_000001_init"), None);
        assert_eq!(version_of("m20260520_000001_init"), Some(20260520000001));
    }
}

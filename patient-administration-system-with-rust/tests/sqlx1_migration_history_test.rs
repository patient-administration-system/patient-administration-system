//! SQLX-1: moving to sqlx migrations keeps the history. A database SeaORM migrated is adopted, never migrated twice.
//! Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test makes its own empty database on the server `DATABASE_URL` points at, and fakes the old record: it
//! applies migrations, moves the record into a `seaql_migrations` table of the shape SeaORM wrote, and empties
//! `_sqlx_migrations`. Real databases were checked by hand too: see `plan.md` §17.

mod common;

use sqlx::migrate::MigrationType;
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn with_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database name");
    format!("{base}/{name}")
}

async fn with_empty_database<F: AsyncFnOnce(&PgPool)>(work: F) {
    let url = common::database_url().unwrap();
    let name = format!("pas_sqlx1_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&url).await.unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .await
        .unwrap();
    let pool = PgPool::connect(&with_database(&url, &name)).await.unwrap();
    work(&pool).await;
    pool.close().await;
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .await
        .unwrap();
}

fn ups() -> Vec<(i64, String)> {
    migration::MIGRATOR
        .iter()
        .filter(|m| !matches!(m.migration_type, MigrationType::ReversibleDown))
        .map(|m| (m.version, m.description.to_string()))
        .collect()
}

/// The name SeaORM recorded: `20260520000001`, `init` becomes `m20260520_000001_init`.
fn seaorm_name(version: i64, description: &str) -> String {
    let v = version.to_string();
    let (date, sequence) = v.split_at(8);
    format!("m{date}_{sequence}_{}", description.replace(' ', "_"))
}

/// Apply the first `count` migrations and leave the record as SeaORM would have.
async fn as_seaorm_left_it(pool: &PgPool, count: usize) {
    let all = ups();
    migration::up_to(pool, all[count - 1].0).await.unwrap();
    pool.execute(
        "CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL)",
    )
    .await
    .unwrap();
    for (version, description) in &all[..count] {
        record(pool, &seaorm_name(*version, description)).await;
    }
    pool.execute("DELETE FROM _sqlx_migrations").await.unwrap();
}

async fn record(pool: &PgPool, name: &str) {
    sqlx::query("INSERT INTO seaql_migrations VALUES ($1, 0)")
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
}

async fn adopted(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_database_seaorm_finished_is_adopted_and_nothing_runs_twice() {
    with_empty_database(async |pool| {
        as_seaorm_left_it(pool, ups().len()).await;
        assert_eq!(migration::adopt(pool).await.unwrap(), ups().len() as u64);
        migration::up(pool).await.unwrap();

        let status = migration::status(pool).await.unwrap();
        assert_eq!(status.len(), ups().len());
        assert!(status.iter().all(|m| m.applied));
        let times: i64 = sqlx::query_scalar("SELECT max(execution_time) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await
            .unwrap();
        // sqlx also accepted every checksum: `up` fails on a mismatch.
        assert_eq!(times, 0, "every row came from adoption: none was executed");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_database_part_way_through_is_adopted_and_finished() {
    with_empty_database(async |pool| {
        as_seaorm_left_it(pool, 40).await;
        migration::up(pool).await.unwrap();
        assert!(
            migration::status(pool)
                .await
                .unwrap()
                .iter()
                .all(|m| m.applied)
        );
        let executed: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE execution_time > 0")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(executed as usize, ups().len() - 40, "only the rest ran");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_database_newer_than_this_binary_is_refused_and_left_alone() {
    with_empty_database(async |pool| {
        as_seaorm_left_it(pool, ups().len()).await;
        record(pool, "m20991231_000099_from_the_future").await;
        let refused = migration::up(pool).await.unwrap_err().to_string();
        assert!(refused.contains("newer than this binary"), "{refused}");
        assert_eq!(adopted(pool).await, 0, "nothing carried over");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_record_with_a_gap_is_refused_and_left_alone() {
    with_empty_database(async |pool| {
        as_seaorm_left_it(pool, 10).await;
        let (version, description) = &ups()[3];
        sqlx::query("DELETE FROM seaql_migrations WHERE version = $1")
            .bind(seaorm_name(*version, description))
            .execute(pool)
            .await
            .unwrap();
        let refused = migration::up(pool).await.unwrap_err().to_string();
        assert!(refused.contains("gap"), "{refused}");
        assert_eq!(adopted(pool).await, 0, "nothing carried over");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_migration_reverted_after_adoption_is_not_marked_applied_again() {
    with_empty_database(async |pool| {
        as_seaorm_left_it(pool, ups().len()).await;
        migration::up(pool).await.unwrap();
        migration::down(pool, 1).await.unwrap();
        assert_eq!(
            migration::adopt(pool).await.unwrap(),
            0,
            "adopted only once"
        );
        let status = migration::status(pool).await.unwrap();
        assert!(
            !status.last().unwrap().applied,
            "the old record must not bring it back"
        );
        migration::up(pool).await.unwrap();
        assert!(
            migration::status(pool)
                .await
                .unwrap()
                .iter()
                .all(|m| m.applied)
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_empty_database_needs_no_adoption() {
    with_empty_database(async |pool| {
        assert_eq!(migration::adopt(pool).await.unwrap(), 0);
        migration::up(pool).await.unwrap();
        assert!(
            migration::status(pool)
                .await
                .unwrap()
                .iter()
                .all(|m| m.applied)
        );
    })
    .await;
}

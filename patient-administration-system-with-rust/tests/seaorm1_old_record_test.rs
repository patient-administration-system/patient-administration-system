//! SEAORM-1: nothing of SeaORM is left in a database. Migration 61 drops `seaql_migrations`, the record SeaORM kept,
//! which `pas-migrate` once carried into `_sqlx_migrations` (SQLX-1) and never read again. Needs PostgreSQL:
//! `cargo test -- --ignored`.
//!
//! Each test makes its own empty database on the server `DATABASE_URL` points at.

mod common;

use sqlx::{Connection, Executor, PgConnection, PgPool};

/// The migration before the one that drops the old record.
const BEFORE_THE_DROP: i64 = 20261110000060;

fn with_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database name");
    format!("{base}/{name}")
}

async fn with_empty_database<F: AsyncFnOnce(&PgPool)>(work: F) {
    let url = common::database_url().unwrap();
    let name = format!("pas_seaorm1_{}", uuid::Uuid::new_v4().simple());
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

async fn has_the_old_record(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('seaql_migrations')::text")
        .fetch_one(pool)
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_database_that_kept_seaorms_record_loses_it_and_keeps_its_own() {
    with_empty_database(async |pool| {
        migration::up_to(pool, BEFORE_THE_DROP).await.unwrap();
        // As SQLX-1 left a database SeaORM had migrated: the old record beside the new one.
        sqlx::raw_sql(
            "CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL); \
             INSERT INTO seaql_migrations VALUES ('m20260520_000001_init', 0)",
        )
        .execute(pool)
        .await
        .unwrap();

        migration::up(pool).await.unwrap();

        assert!(!has_the_old_record(pool).await);
        let status = migration::status(pool).await.unwrap();
        assert!(status.iter().all(|m| m.applied), "{status:?}");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_fresh_database_migrates_and_never_had_the_old_record() {
    with_empty_database(async |pool| {
        migration::up(pool).await.unwrap();
        assert!(!has_the_old_record(pool).await);
        // Taking the drop back down restores nothing and fails nothing.
        migration::down(pool, 1).await.unwrap();
        assert!(!has_the_old_record(pool).await);
        migration::up(pool).await.unwrap();
    })
    .await;
}

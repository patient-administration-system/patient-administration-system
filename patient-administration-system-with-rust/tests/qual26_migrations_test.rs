//! QUAL-26: every migration after the last forward-only one can be undone and done again. Needs
//! PostgreSQL: `cargo test -- --ignored`.
//!
//! The test makes its own empty databases on the server `DATABASE_URL` points at. One is migrated all the
//! way, taken down to just after `m20260601_000006_delegate_identity` (forward-only by design: undoing it
//! would lose identity data) and migrated up again; the other is migrated only to that point. A `down`
//! that does nothing leaves a column, index or constraint behind, so the schemas differ or the second
//! `up` fails on it.

mod common;

use migration::{Migrator, MigratorTrait};
use patient_administration_system::db::connect;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement};

/// The last migration whose `down` refuses by design.
const FORWARD_ONLY: &str = "m20260601_000006_delegate_identity";

/// `DATABASE_URL` with its database name replaced.
fn with_database(url: &str, name: &str) -> String {
    let (base, _) = url
        .rsplit_once('/')
        .expect("DATABASE_URL has a database name");
    let query = url.split_once('?').map(|(_, q)| format!("?{q}"));
    format!("{base}/{name}{}", query.unwrap_or_default())
}

/// Every column, constraint and index in `public`, one sorted line each, without the migrations table.
async fn schema(db: &DatabaseConnection) -> Result<Vec<String>, DbErr> {
    let sql = "SELECT 'column ' || table_name || '.' || column_name || ' ' || data_type || ' ' || is_nullable \
                      || ' ' || coalesce(column_default, '') AS line \
                 FROM information_schema.columns \
                WHERE table_schema = 'public' AND table_name <> 'seaql_migrations' \
               UNION ALL \
               SELECT 'constraint ' || conrelid::regclass || ' ' || conname || ' ' || pg_get_constraintdef(oid) \
                 FROM pg_constraint \
                WHERE connamespace = 'public'::regnamespace AND conrelid <> 'seaql_migrations'::regclass \
               UNION ALL \
               SELECT 'index ' || tablename || ' ' || indexdef \
                 FROM pg_indexes \
                WHERE schemaname = 'public' AND tablename <> 'seaql_migrations' \
               ORDER BY 1";
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await?
        .iter()
        .map(|r| r.try_get::<String>("", "line"))
        .collect()
}

async fn with_fresh_database<T>(
    url: &str,
    work: impl AsyncFnOnce(&DatabaseConnection) -> Result<T, DbErr>,
) -> T {
    let admin = connect(url).await.expect("connect");
    let name = format!("pas_qual26_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute_unprepared(&format!("CREATE DATABASE {name}"))
        .await
        .expect("create database");
    let result = async {
        let db = Database::connect(with_database(url, &name)).await?;
        let out = work(&db).await;
        db.close().await?;
        out
    }
    .await;
    admin
        .execute_unprepared(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await
        .expect("drop database");
    result.unwrap_or_else(|e| panic!("on a fresh database: {e}"))
}

#[tokio::test]
#[ignore]
async fn migrations_after_the_forward_only_one_go_down_and_up_again_and_leave_the_same_schema() {
    let url = common::database_url().unwrap();
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_string())
        .collect();
    let keep = 1 + names
        .iter()
        .position(|n| n == FORWARD_ONLY)
        .expect("the forward-only migration is listed");
    let undo = (names.len() - keep) as u32;
    assert!(undo > 30, "only {undo} migrations would be exercised");

    let base = with_fresh_database(&url, async |db| {
        Migrator::up(db, Some(keep as u32)).await?;
        schema(db).await
    })
    .await;

    let (full, down, again) = with_fresh_database(&url, async |db| {
        Migrator::up(db, None).await?;
        let full = schema(db).await?;
        Migrator::down(db, Some(undo)).await?;
        let down = schema(db).await?;
        Migrator::up(db, None).await?;
        Ok((full, down, schema(db).await?))
    })
    .await;

    let diff = |a: &[String], b: &[String]| -> Vec<String> {
        let mut d: Vec<String> = a
            .iter()
            .filter(|l| !b.contains(l))
            .map(|l| format!("- {l}"))
            .collect();
        d.extend(
            b.iter()
                .filter(|l| !a.contains(l))
                .map(|l| format!("+ {l}")),
        );
        d
    };
    assert_eq!(
        diff(&base, &down),
        Vec::<String>::new(),
        "after down, the schema is not what migrating only to {FORWARD_ONLY} gives"
    );
    assert_eq!(
        diff(&full, &again),
        Vec::<String>::new(),
        "migrating up again does not rebuild the same schema"
    );
}

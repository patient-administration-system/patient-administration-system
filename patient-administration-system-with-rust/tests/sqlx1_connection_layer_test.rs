//! SQLX-1: what the project's connection layer (`db::sql`) promises. Stage 4 swaps what is underneath it, and this
//! test is the proof that the swap changed nothing, so it pins behaviour, not implementation. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{DateTime, Utc};
use patient_administration_system::db::connect;
use patient_administration_system::db::entities::outbox_dead_letter;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{
    Conn, Pool, Stmt, Table, Transact, is_rejected_statement, is_unique_violation,
};
use patient_administration_system::{Error, Result};
use uuid::Uuid;

/// A table of its own, so these tests do not touch the schema or each other.
async fn table(pool: &Pool) -> String {
    let name = format!("sqlx1_{}", Uuid::new_v4().simple());
    pool.batch(&format!(
        "CREATE TABLE {name} (id uuid PRIMARY KEY, n integer NOT NULL CHECK (n >= 0), \
         big bigint, ok boolean, ratio double precision, words text[], doc jsonb, at timestamptz, note text)"
    ))
    .await
    .unwrap();
    name
}

async fn drop_table(pool: &Pool, name: &str) {
    pool.batch(&format!("DROP TABLE {name}")).await.unwrap();
}

async fn pool() -> Pool {
    connect(&common::database_url().unwrap()).await.unwrap()
}

async fn count(pool: &Pool, table: &str) -> i64 {
    pool.one(Stmt::text(format!("SELECT count(*) AS n FROM {table}")))
        .await
        .unwrap()
        .unwrap()
        .col("n")
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_transaction_commits_on_ok_and_rolls_back_on_err() {
    let pool = pool().await;
    let t = table(&pool).await;

    let insert = format!("INSERT INTO {t} (id, n) VALUES ($1, 1)");
    let sql = insert.clone();
    pool.in_txn::<_, ()>(|txn| {
        Box::pin(async move {
            txn.run(Stmt::new(sql, [Uuid::new_v4().into()])).await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    assert_eq!(count(&pool, &t).await, 1, "committed");

    let sql = insert.clone();
    let failed: Result<()> = pool
        .in_txn::<_, ()>(|txn| {
            Box::pin(async move {
                txn.run(Stmt::new(sql, [Uuid::new_v4().into()])).await?;
                Err(Error::conflict("changed my mind"))
            })
        })
        .await;
    assert!(
        matches!(failed, Err(Error::Conflict(_))),
        "the closure's own error comes back"
    );
    assert_eq!(count(&pool, &t).await, 1, "rolled back");

    let by_hand = pool.begin_txn().await.unwrap();
    by_hand
        .run(Stmt::new(insert, [Uuid::new_v4().into()]))
        .await
        .unwrap();
    by_hand.commit().await.unwrap();
    assert_eq!(
        count(&pool, &t).await,
        2,
        "a transaction begun by hand commits too"
    );
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_failed_statement_in_a_transaction_undoes_what_came_before_it() {
    let pool = pool().await;
    let t = table(&pool).await;
    let (first, second) = (
        format!("INSERT INTO {t} (id, n) VALUES ($1, 1)"),
        format!("INSERT INTO {t} (id, n) VALUES ($1, -1)"),
    );
    let failed = pool
        .in_txn::<_, ()>(|txn| {
            Box::pin(async move {
                txn.run(Stmt::new(first, [Uuid::new_v4().into()])).await?;
                txn.run(Stmt::new(second, [Uuid::new_v4().into()])).await?;
                Ok(())
            })
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(count(&pool, &t).await, 0);
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_kind_of_value_goes_in_and_comes_back_as_it_was() {
    let pool = pool().await;
    let t = table(&pool).await;
    let id = Uuid::new_v4();
    let at: DateTime<Utc> = "2026-11-06T10:00:00.123456Z".parse().unwrap();
    let doc = serde_json::json!({ "a": [1, 2], "b": "text" });
    let words = vec!["one".to_string(), "two".to_string()];
    pool.run(Stmt::new(
        format!(
            "INSERT INTO {t} (id, n, big, ok, ratio, words, doc, at, note) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
        ),
        [
            id.into(),
            7i32.into(),
            9_000_000_000i64.into(),
            true.into(),
            0.25f64.into(),
            words.clone().into(),
            doc.clone().into(),
            at.into(),
            None::<String>.into(),
        ],
    ))
    .await
    .unwrap();
    let row = pool
        .one(Stmt::new(
            format!("SELECT * FROM {t} WHERE id = $1"),
            [id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.col::<Uuid>("id").unwrap(), id);
    assert_eq!(row.col::<i32>("n").unwrap(), 7);
    assert_eq!(row.col::<i64>("big").unwrap(), 9_000_000_000);
    assert!(row.col::<bool>("ok").unwrap());
    assert_eq!(row.col::<f64>("ratio").unwrap(), 0.25);
    assert_eq!(row.col::<Vec<String>>("words").unwrap(), words);
    assert_eq!(row.col::<serde_json::Value>("doc").unwrap(), doc);
    assert_eq!(row.col::<Option<String>>("note").unwrap(), None);
    // `updated_at` to the microsecond is a record's version (NFR-27): no precision may be lost.
    assert_eq!(row.col::<DateTime<Utc>>("at").unwrap(), at);
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_row_is_none_and_a_write_says_how_many_it_changed() {
    let pool = pool().await;
    let t = table(&pool).await;
    let select = format!("SELECT id FROM {t} WHERE id = $1");
    let missing = pool
        .one(Stmt::new(select, [Uuid::new_v4().into()]))
        .await
        .unwrap();
    assert!(missing.is_none());
    assert!(
        pool.all(Stmt::text(format!("SELECT id FROM {t}")))
            .await
            .unwrap()
            .is_empty()
    );
    for _ in 0..3 {
        pool.run(Stmt::new(
            format!("INSERT INTO {t} (id, n) VALUES ($1, 1)"),
            [Uuid::new_v4().into()],
        ))
        .await
        .unwrap();
    }
    let done = pool
        .run(Stmt::text(format!("UPDATE {t} SET n = 2")))
        .await
        .unwrap();
    assert_eq!(done.rows_affected(), 3);
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_refusal_is_told_apart_from_a_failure() {
    let pool = pool().await;
    let t = table(&pool).await;
    let id = Uuid::new_v4();
    let insert = format!("INSERT INTO {t} (id, n) VALUES ($1, $2)");
    pool.run(Stmt::new(insert.clone(), [id.into(), 1i32.into()]))
        .await
        .unwrap();

    let duplicate = pool
        .run(Stmt::new(insert.clone(), [id.into(), 1i32.into()]))
        .await
        .unwrap_err();
    assert!(is_unique_violation(&duplicate), "{duplicate:?}");
    assert!(is_rejected_statement(&duplicate), "{duplicate:?}");

    let check = pool
        .run(Stmt::new(insert, [Uuid::new_v4().into(), (-1i32).into()]))
        .await
        .unwrap_err();
    assert!(!is_unique_violation(&check), "{check:?}");
    assert!(is_rejected_statement(&check), "{check:?}");

    assert!(!is_unique_violation(&Error::conflict(
        "not from the database"
    )));
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_wrong_column_names_the_column_and_stays_a_withheld_database_error() {
    let pool = pool().await;
    let row = pool
        .one(Stmt::text("SELECT 1 AS present"))
        .await
        .unwrap()
        .unwrap();
    let e = row.col::<i32>("absent").unwrap_err();
    assert!(matches!(e, Error::Database(_)), "{e:?}");
    assert!(e.to_string().contains("absent"), "{e}");
    let told = e.client_message();
    assert!(told.starts_with("Database error (reference "), "{told}");
    assert!(
        !told.contains("absent"),
        "the caller is not told the column: {told}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_batch_may_hold_several_statements() {
    let pool = pool().await;
    let t = table(&pool).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    pool.batch(&format!(
        "INSERT INTO {t} (id, n) VALUES ('{a}', 1); INSERT INTO {t} (id, n) VALUES ('{b}', 2);"
    ))
    .await
    .unwrap();
    assert_eq!(count(&pool, &t).await, 2);
    drop_table(&pool, &t).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_row_locked_in_a_transaction_makes_a_second_transaction_wait() {
    let pool = pool().await;
    let t = table(&pool).await;
    let id = Uuid::new_v4();
    pool.run(Stmt::new(
        format!("INSERT INTO {t} (id, n) VALUES ($1, 0)"),
        [id.into()],
    ))
    .await
    .unwrap();
    let bump = |t: String, pool: Pool| async move {
        pool.in_txn::<_, ()>(move |txn| {
            Box::pin(async move {
                let row = txn
                    .one(Stmt::new(
                        format!("SELECT n FROM {t} WHERE id = $1 FOR UPDATE"),
                        [id.into()],
                    ))
                    .await?
                    .ok_or_else(|| Error::not_found("row"))?;
                let n: i32 = row.col("n")?;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                txn.run(Stmt::new(
                    format!("UPDATE {t} SET n = $2 WHERE id = $1"),
                    [id.into(), (n + 1).into()],
                ))
                .await?;
                Ok(())
            })
        })
        .await
    };
    let (a, b) = tokio::join!(bump(t.clone(), pool.clone()), bump(t.clone(), pool.clone()));
    a.unwrap();
    b.unwrap();
    let row = pool
        .one(Stmt::text(format!("SELECT n FROM {t}")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.col::<i32>("n").unwrap(), 2, "neither update was lost");
    drop_table(&pool, &t).await;
}

fn dead_letter() -> outbox_dead_letter::Model {
    let now = "2026-11-06T10:00:00.123456+00:00".parse().unwrap();
    outbox_dead_letter::Model {
        id: Uuid::new_v4(),
        original_id: Uuid::new_v4(),
        event_type: "sqlx1.test".into(),
        payload: serde_json::json!({ "k": 1 }),
        created_at: now,
        dead_lettered_at: now,
        retry_count: 1,
        last_error: "first".into(),
    }
}

async fn forget(pool: &Pool, id: Uuid) {
    pool.run(Stmt::new(
        "DELETE FROM outbox_dead_letters WHERE id = $1",
        [id.into()],
    ))
    .await
    .unwrap();
}

/// The lost update this project has fixed route by route (OFH-11, hazard H-15): a patch writes the columns it names
/// and no others, so what someone else changed since it was read stays.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patch_leaves_a_column_someone_else_changed_since_it_was_read() {
    let pool = pool().await;
    let read = dead_letter().insert(&pool).await.unwrap();

    pool.run(Stmt::new(
        "UPDATE outbox_dead_letters SET retry_count = 9 WHERE id = $1",
        [read.id.into()],
    ))
    .await
    .unwrap();

    let mut patch = read.patch();
    patch.set("last_error", "second");
    let stored = patch.update(&pool).await.unwrap();
    assert_eq!(stored.last_error, "second");
    assert_eq!(stored.retry_count, 9, "the other writer's change was kept");
    assert_eq!(read.retry_count, 1, "and the copy that was read was stale");
    forget(&pool, read.id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn insert_returns_the_stored_row_and_update_row_keeps_created_at() {
    let pool = pool().await;
    let row = dead_letter();
    let stored = row.insert(&pool).await.unwrap();
    assert_eq!(
        stored, row,
        "nothing changed on the way in or out, to the microsecond"
    );

    let mut changed = stored.clone();
    changed.last_error = "second".into();
    changed.created_at = "2000-01-01T00:00:00+00:00".parse().unwrap();
    let written = changed.update_row(&pool).await.unwrap();
    assert_eq!(written.last_error, "second");
    assert_eq!(
        written.created_at, row.created_at,
        "created_at is never rewritten"
    );
    forget(&pool, row.id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patch_of_a_row_that_has_gone_is_an_error_not_a_silent_success() {
    let pool = pool().await;
    let row = dead_letter().insert(&pool).await.unwrap();
    forget(&pool, row.id).await;
    let mut patch = row.patch();
    patch.set("last_error", "second");
    assert!(patch.update(&pool).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn inserting_a_row_twice_is_a_unique_violation() {
    let pool = pool().await;
    let row = dead_letter().insert(&pool).await.unwrap();
    let again = row.insert(&pool).await.unwrap_err();
    assert!(is_unique_violation(&again), "{again:?}");
    forget(&pool, row.id).await;
}

async fn columns_exist<T: Table>(pool: &Pool) {
    let sql = format!("SELECT {} FROM {} LIMIT 0", T::COLUMNS.join(", "), T::NAME);
    pool.all(Stmt::text(sql))
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", T::NAME));
    assert_eq!(T::COLUMNS[0], "id", "{}", T::NAME);
}

/// Columns are named in text now, so nothing checks them at compile time. This does, for every model.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_model_names_columns_its_table_has() {
    use patient_administration_system::db::entities::*;
    let pool = pool().await;
    columns_exist::<account::Model>(&pool).await;
    columns_exist::<admission::Model>(&pool).await;
    columns_exist::<appointment::Model>(&pool).await;
    columns_exist::<appointment_series::Model>(&pool).await;
    columns_exist::<audit_log::Model>(&pool).await;
    columns_exist::<bed::Model>(&pool).await;
    columns_exist::<bed_assignment::Model>(&pool).await;
    columns_exist::<charge::Model>(&pool).await;
    columns_exist::<consent::Model>(&pool).await;
    columns_exist::<coverage::Model>(&pool).await;
    columns_exist::<discharge::Model>(&pool).await;
    columns_exist::<encounter::Model>(&pool).await;
    columns_exist::<facility::Model>(&pool).await;
    columns_exist::<generated_letter::Model>(&pool).await;
    columns_exist::<invoice::Model>(&pool).await;
    columns_exist::<letter_template::Model>(&pool).await;
    columns_exist::<outbox_dead_letter::Model>(&pool).await;
    columns_exist::<outbox_event::Model>(&pool).await;
    columns_exist::<payment::Model>(&pool).await;
    columns_exist::<referral::Model>(&pool).await;
    columns_exist::<room::Model>(&pool).await;
    columns_exist::<rtt_clock_event::Model>(&pool).await;
    columns_exist::<rtt_pathway::Model>(&pool).await;
    columns_exist::<schedule::Model>(&pool).await;
    columns_exist::<slot::Model>(&pool).await;
    columns_exist::<transfer::Model>(&pool).await;
    columns_exist::<waitlist_entry::Model>(&pool).await;
    columns_exist::<ward::Model>(&pool).await;
}

//! SQLX-1: what the project's connection layer (`db::sql`) promises. Stage 4 swaps what is underneath it, and this
//! test is the proof that the swap changed nothing, so it pins behaviour, not implementation. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{DateTime, Utc};
use patient_administration_system::db::connect;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{
    Conn, Pool, Stmt, Transact, is_rejected_statement, is_unique_violation,
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

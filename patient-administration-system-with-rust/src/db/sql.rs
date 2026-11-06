//! The project's own way to talk to the database (SQLX-1, `plan.md` §17).
//!
//! Every query in the system goes through [`Conn`]: a statement ([`Stmt`]) with bound values ([`Param`]), run on a
//! pool or a transaction, giving rows ([`Row`]). Nothing outside this module and `db::row` needs to know what is
//! underneath, which is the point: today that is sea-orm, and swapping it for sqlx happens here, in one place,
//! because a sea-orm transaction cannot be handed to sqlx and the audit and outbox writes run inside every
//! transaction.
//!
//! [`Conn`] takes `&self`, for a pool and a transaction alike, so a repository function is written once and a
//! transaction can be shared by reference inside its closure.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use uuid::Uuid;

use crate::db::row::RowExt;
use crate::{Error, Result};

/// The connection pool.
pub type Pool = sea_orm::DatabaseConnection;

/// An open transaction, as a closure given to [`Transact::in_txn`] receives it.
pub type Txn = sea_orm::DatabaseTransaction;

/// One bound value: anything with `.into()` at the call site.
#[derive(Debug, Clone)]
pub struct Param(sea_orm::Value);

impl<T: Into<sea_orm::Value>> From<T> for Param {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

/// A statement and its bound values. Values are always bound, never written into the text.
#[derive(Debug, Clone)]
pub struct Stmt {
    sql: String,
    params: Vec<Param>,
}

impl Stmt {
    /// A statement with `$1`, `$2`, ... bound to `params`, in order.
    pub fn new(sql: impl Into<String>, params: impl IntoIterator<Item = Param>) -> Self {
        Self {
            sql: sql.into(),
            params: params.into_iter().collect(),
        }
    }

    /// A statement with nothing to bind.
    pub fn text(sql: impl Into<String>) -> Self {
        Self::new(sql, [])
    }

    fn into_statement(self) -> Statement {
        Statement::from_sql_and_values(
            DbBackend::Postgres,
            self.sql,
            self.params.into_iter().map(|p| p.0),
        )
    }
}

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Done {
    rows_affected: u64,
}

impl Done {
    /// How many rows it changed.
    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }
}

/// One row of a result. Read it with [`crate::db::row::RowExt::col`].
#[derive(Debug)]
pub struct Row(pub(super) sea_orm::QueryResult);

/// A pool or a transaction.
pub trait Conn: ConnectionTrait + Sync {
    /// Run a statement that returns no rows.
    fn run(&self, stmt: Stmt) -> impl Future<Output = Result<Done>> + Send;
    /// The first row, if there is one.
    fn one(&self, stmt: Stmt) -> impl Future<Output = Result<Option<Row>>> + Send;
    /// Every row.
    fn all(&self, stmt: Stmt) -> impl Future<Output = Result<Vec<Row>>> + Send;
    /// The first row as `T`, if there is one.
    fn one_as<T: FromRow + Send>(
        &self,
        stmt: Stmt,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move {
            match self.one(stmt).await? {
                Some(row) => Ok(Some(T::from_row(&row)?)),
                None => Ok(None),
            }
        }
    }
    /// Every row as `T`.
    fn all_as<T: FromRow + Send>(&self, stmt: Stmt) -> impl Future<Output = Result<Vec<T>>> + Send {
        async move {
            let rows = self.all(stmt).await?;
            rows.iter().map(T::from_row).collect()
        }
    }
    /// The number in column `n` of the one row a `SELECT count(*) AS n` returns.
    fn count(&self, stmt: Stmt) -> impl Future<Output = Result<u64>> + Send {
        async move {
            let row = self.one(stmt).await?.ok_or_else(no_row)?;
            let n: i64 = row.col("n")?;
            Ok(n.max(0) as u64)
        }
    }
    /// Run text as it stands, which may hold several statements. It binds nothing, so it must hold no outside value.
    fn batch(&self, sql: &str) -> impl Future<Output = Result<Done>> + Send;
}

impl<C: ConnectionTrait + Sync> Conn for C {
    async fn run(&self, stmt: Stmt) -> Result<Done> {
        let done = self
            .execute_raw(stmt.into_statement())
            .await
            .map_err(Error::Database)?;
        Ok(Done {
            rows_affected: done.rows_affected(),
        })
    }

    async fn one(&self, stmt: Stmt) -> Result<Option<Row>> {
        let row = self
            .query_one_raw(stmt.into_statement())
            .await
            .map_err(Error::Database)?;
        Ok(row.map(Row))
    }

    async fn all(&self, stmt: Stmt) -> Result<Vec<Row>> {
        let rows = self
            .query_all_raw(stmt.into_statement())
            .await
            .map_err(Error::Database)?;
        Ok(rows.into_iter().map(Row).collect())
    }

    async fn batch(&self, sql: &str) -> Result<Done> {
        let done = self
            .execute_unprepared(sql)
            .await
            .map_err(Error::Database)?;
        Ok(Done {
            rows_affected: done.rows_affected(),
        })
    }
}

/// A statement that had to return a row returned none.
fn no_row() -> Error {
    Error::internal("the database returned no row where one was expected")
}

/// A type read from a row.
pub trait FromRow: Sized {
    /// Read it; the error names any column that does not fit.
    fn from_row(row: &Row) -> Result<Self>;
}

/// The columns of a table, in a fixed order, and a value for each. A row's `id` is its primary key.
pub trait Table: FromRow + Send + Sync + 'static {
    /// The table's name.
    const NAME: &'static str;
    /// Every column this type holds. `id` is first.
    const COLUMNS: &'static [&'static str];
    /// The primary key.
    fn id(&self) -> Uuid;
    /// A value for each of [`Table::COLUMNS`], in the same order.
    fn values(&self) -> Vec<Param>;

    /// Write this as a new row, and return what is stored.
    fn insert<C: Conn>(&self, conn: &C) -> impl Future<Output = Result<Self>> + Send {
        async move {
            let holes: Vec<String> = (1..=Self::COLUMNS.len()).map(|n| format!("${n}")).collect();
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                Self::NAME,
                Self::COLUMNS.join(", "),
                holes.join(", ")
            );
            conn.one_as::<Self>(Stmt::new(sql, self.values()))
                .await?
                .ok_or_else(no_row)
        }
    }

    /// Write every column of this row except `id` and `created_at`, and return what is stored.
    ///
    /// This overwrites the whole row, so use it only with the row locked, or when replacing it is the intent. To
    /// change some columns of a row, use [`Table::patch`], which leaves the others alone.
    fn update_row<C: Conn>(&self, conn: &C) -> impl Future<Output = Result<Self>> + Send {
        async move {
            let mut patch = self.patch();
            for (column, value) in Self::COLUMNS.iter().zip(self.values()) {
                if *column != "id" && *column != "created_at" {
                    patch.columns.push((column, value));
                }
            }
            patch.update(conn).await
        }
    }

    /// Start changing some columns of this row.
    fn patch(&self) -> Patch<Self> {
        Patch {
            id: self.id(),
            columns: Vec::new(),
            table: PhantomData,
        }
    }
}

/// Some columns of one row to change. Only the columns named with [`Patch::set`] are written, so a change made by
/// someone else to any other column stays.
#[derive(Debug)]
pub struct Patch<T> {
    id: Uuid,
    columns: Vec<(&'static str, Param)>,
    table: PhantomData<T>,
}

impl<T: Table> Patch<T> {
    /// Write `value` to `column`.
    pub fn set(&mut self, column: &'static str, value: impl Into<Param>) {
        self.columns.push((column, value.into()));
    }

    /// Write the named columns and return the stored row. A row that has gone is an error.
    pub async fn update<C: Conn>(self, conn: &C) -> Result<T> {
        let assignments: Vec<String> = self
            .columns
            .iter()
            .enumerate()
            .map(|(n, (column, _))| format!("{column} = ${}", n + 2))
            .collect();
        let sql = format!(
            "UPDATE {} SET {} WHERE id = $1 RETURNING *",
            T::NAME,
            assignments.join(", ")
        );
        let mut params = vec![Param::from(self.id)];
        params.extend(self.columns.into_iter().map(|(_, value)| value));
        conn.one_as::<T>(Stmt::new(sql, params))
            .await?
            .ok_or_else(no_row)
    }
}

/// The boxed future a transaction closure returns.
pub type TxnFuture<'c, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'c>>;

/// Run a closure in a transaction: it commits when the closure returns `Ok`, and rolls back on `Err`.
pub trait Transact {
    /// Begin a transaction by hand, for work that cannot be one closure. Prefer [`Transact::in_txn`].
    fn begin_txn(&self) -> impl Future<Output = Result<Txn>> + Send;

    /// See the trait.
    fn in_txn<F, T>(&self, work: F) -> impl Future<Output = Result<T>> + Send
    where
        F: for<'c> FnOnce(&'c Txn) -> TxnFuture<'c, T> + Send,
        T: Send;
}

impl Transact for Pool {
    async fn begin_txn(&self) -> Result<Txn> {
        self.begin().await.map_err(Error::Database)
    }

    async fn in_txn<F, T>(&self, work: F) -> Result<T>
    where
        F: for<'c> FnOnce(&'c Txn) -> TxnFuture<'c, T> + Send,
        T: Send,
    {
        let txn = self.begin().await.map_err(Error::Database)?;
        match work(&txn).await {
            Ok(value) => {
                txn.commit().await.map_err(Error::Database)?;
                Ok(value)
            }
            Err(e) => {
                // A failed rollback leaves nothing to do: the connection closing rolls back too.
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }
}

/// Did the database refuse this because of a unique constraint?
pub fn is_unique_violation(error: &Error) -> bool {
    matches!(
        error,
        Error::Database(e) if matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_)))
    )
}

/// Did the server reject the statement itself (a check constraint, say), rather than the connection fail?
pub fn is_rejected_statement(error: &Error) -> bool {
    matches!(
        error,
        Error::Database(sea_orm::DbErr::Exec(_) | sea_orm::DbErr::Query(_))
    )
}

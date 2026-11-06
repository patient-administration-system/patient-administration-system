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
use std::pin::Pin;

use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};

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

    /// For the few sea-orm `FromQueryResult` reads left. Stage 3 of SQLX-1 removes them, and this.
    pub(crate) fn raw(self) -> Statement {
        self.into_statement()
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

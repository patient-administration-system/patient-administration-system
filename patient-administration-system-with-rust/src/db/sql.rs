//! The project's own way to talk to the database (SQLX-1, `plan.md` §17).
//!
//! Every query in the system goes through [`Conn`]: a statement ([`Stmt`]) with bound values ([`Param`]), run on a
//! pool or a transaction, giving rows ([`Row`]). Queries are checked at run time, and `tests/qual7_sql_prepares_test.rs`
//! prepares every literal statement against the migrated schema.
//!
//! [`Conn`] takes `&self`, for a pool and a transaction alike, so a repository function is written once and a
//! transaction can be shared by reference inside its closure. A sqlx transaction needs `&mut` to run a statement, so
//! [`Txn`] holds it behind a lock: statements in one transaction run one at a time, as they must on one connection.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::Query;
use sqlx::{Arguments, AssertSqlSafe, Executor, Postgres};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::row::RowExt;
use crate::{Error, Result};

/// The connection pool.
pub type Pool = sqlx::PgPool;

/// An open transaction, as a closure given to [`Transact::in_txn`] receives it. A transaction dropped without a
/// commit rolls back, which covers a panic as well as an early return.
#[derive(Debug)]
pub struct Txn(Mutex<sqlx::Transaction<'static, Postgres>>);

impl Txn {
    /// Make the transaction's work permanent.
    pub async fn commit(self) -> std::result::Result<(), sqlx::Error> {
        self.0.into_inner().commit().await
    }

    /// Undo the transaction's work.
    pub async fn rollback(self) -> std::result::Result<(), sqlx::Error> {
        self.0.into_inner().rollback().await
    }
}

/// One bound value: anything with `.into()` at the call site. A `None` still has its type, so the server knows it.
#[derive(Debug, Clone)]
pub enum Param {
    Uuid(Option<Uuid>),
    Text(Option<String>),
    Int2(Option<i16>),
    Int4(Option<i32>),
    Int8(Option<i64>),
    Float8(Option<f64>),
    Bool(Option<bool>),
    Time(Option<DateTime<Utc>>),
    Date(Option<NaiveDate>),
    TimeOfDay(Option<NaiveTime>),
    Json(Option<serde_json::Value>),
    TextArray(Option<Vec<String>>),
    UuidArray(Option<Vec<Uuid>>),
}

/// `impl From<T>` and `From<Option<T>>` for a [`Param`] variant, through a conversion to what the variant holds.
macro_rules! param_from {
    ($($t:ty => $variant:ident($into:expr)),* $(,)?) => {$(
        impl From<$t> for Param {
            fn from(value: $t) -> Self {
                Param::$variant(Some($into(value)))
            }
        }
        impl From<Option<$t>> for Param {
            fn from(value: Option<$t>) -> Self {
                Param::$variant(value.map($into))
            }
        }
    )*};
}

param_from! {
    Uuid => Uuid(|v| v),
    String => Text(|v| v),
    &str => Text(|v: &str| v.to_string()),
    &String => Text(|v: &String| v.clone()),
    i16 => Int2(|v| v),
    i32 => Int4(|v| v),
    i64 => Int8(|v| v),
    // Postgres has no unsigned types; a value past i64::MAX is not a count this system can reach.
    u32 => Int8(i64::from),
    u64 => Int8(|v: u64| i64::try_from(v).unwrap_or(i64::MAX)),
    f64 => Float8(|v| v),
    bool => Bool(|v| v),
    DateTime<Utc> => Time(|v| v),
    DateTime<FixedOffset> => Time(|v: DateTime<FixedOffset>| v.to_utc()),
    NaiveDate => Date(|v| v),
    NaiveTime => TimeOfDay(|v| v),
    serde_json::Value => Json(|v| v),
    Vec<String> => TextArray(|v| v),
    Vec<Uuid> => UuidArray(|v| v),
}

impl Param {
    fn bind(self, args: &mut PgArguments) -> std::result::Result<(), sqlx::error::BoxDynError> {
        match self {
            Param::Uuid(v) => args.add(v),
            Param::Text(v) => args.add(v),
            Param::Int2(v) => args.add(v),
            Param::Int4(v) => args.add(v),
            Param::Int8(v) => args.add(v),
            Param::Float8(v) => args.add(v),
            Param::Bool(v) => args.add(v),
            Param::Time(v) => args.add(v),
            Param::Date(v) => args.add(v),
            Param::TimeOfDay(v) => args.add(v),
            Param::Json(v) => args.add(v),
            Param::TextArray(v) => args.add(v),
            Param::UuidArray(v) => args.add(v),
        }
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

    fn into_query(self) -> Result<Query<'static, Postgres, PgArguments>> {
        let mut args = PgArguments::default();
        for param in self.params {
            param
                .bind(&mut args)
                .map_err(|e| Error::Database(sqlx::Error::Encode(e)))?;
        }
        Ok(sqlx::query_with(AssertSqlSafe(self.sql), args))
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
pub struct Row(pub(super) PgRow);

/// A pool or a transaction.
pub trait Conn: Sync {
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

impl Conn for Pool {
    async fn run(&self, stmt: Stmt) -> Result<Done> {
        let done = stmt
            .into_query()?
            .execute(self)
            .await
            .map_err(Error::Database)?;
        Ok(Done {
            rows_affected: done.rows_affected(),
        })
    }

    async fn one(&self, stmt: Stmt) -> Result<Option<Row>> {
        let row = stmt
            .into_query()?
            .fetch_optional(self)
            .await
            .map_err(Error::Database)?;
        Ok(row.map(Row))
    }

    async fn all(&self, stmt: Stmt) -> Result<Vec<Row>> {
        let rows = stmt
            .into_query()?
            .fetch_all(self)
            .await
            .map_err(Error::Database)?;
        Ok(rows.into_iter().map(Row).collect())
    }

    async fn batch(&self, sql: &str) -> Result<Done> {
        let done = self
            .execute(sqlx::raw_sql(AssertSqlSafe(sql.to_string())))
            .await
            .map_err(Error::Database)?;
        Ok(Done {
            rows_affected: done.rows_affected(),
        })
    }
}

impl Conn for Txn {
    async fn run(&self, stmt: Stmt) -> Result<Done> {
        let query = stmt.into_query()?;
        let mut txn = self.0.lock().await;
        let done = query.execute(&mut **txn).await.map_err(Error::Database)?;
        Ok(Done {
            rows_affected: done.rows_affected(),
        })
    }

    async fn one(&self, stmt: Stmt) -> Result<Option<Row>> {
        let query = stmt.into_query()?;
        let mut txn = self.0.lock().await;
        let row = query
            .fetch_optional(&mut **txn)
            .await
            .map_err(Error::Database)?;
        Ok(row.map(Row))
    }

    async fn all(&self, stmt: Stmt) -> Result<Vec<Row>> {
        let query = stmt.into_query()?;
        let mut txn = self.0.lock().await;
        let rows = query.fetch_all(&mut **txn).await.map_err(Error::Database)?;
        Ok(rows.into_iter().map(Row).collect())
    }

    async fn batch(&self, sql: &str) -> Result<Done> {
        let mut txn = self.0.lock().await;
        let done = (&mut **txn)
            .execute(sqlx::raw_sql(AssertSqlSafe(sql.to_string())))
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
        let txn = sqlx::Pool::begin(self).await.map_err(Error::Database)?;
        Ok(Txn(Mutex::new(txn)))
    }

    async fn in_txn<F, T>(&self, work: F) -> Result<T>
    where
        F: for<'c> FnOnce(&'c Txn) -> TxnFuture<'c, T> + Send,
        T: Send,
    {
        let txn = self.begin_txn().await?;
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
    matches!(error, Error::Database(e) if axa_sqlx::is_unique_violation(e))
}

/// Did the server reject the statement itself (a check constraint, say), rather than the connection fail?
pub fn is_rejected_statement(error: &Error) -> bool {
    matches!(error, Error::Database(sqlx::Error::Database(_)))
}

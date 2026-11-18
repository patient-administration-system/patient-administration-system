//! Reading columns out of a row (QUAL-7).
//!
//! `row.col::<T>("name")?` says which column failed. The error is a database error (so it is withheld from callers
//! like any other, see `Error::client_message`) and names the column in the log.

use sqlx::{Postgres, Row as _};

use crate::db::sql::Row;
use crate::{Error, Result};

/// A type a column can be read as: sqlx decodes `Db`, and `from_db` turns it into the type the code uses. For
/// most types they are the same; jiff's times go through jiff-sqlx's wrappers (JIFF-1).
pub trait Column: Sized {
    type Db: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>;
    fn from_db(v: Self::Db) -> Self;
}

/// The types sqlx reads as they are.
macro_rules! column_as_is {
    ($($t:ty),* $(,)?) => {$(
        impl Column for $t {
            type Db = $t;
            fn from_db(v: Self::Db) -> Self {
                v
            }
        }
    )*};
}

column_as_is!(
    String,
    bool,
    i16,
    i32,
    i64,
    f32,
    f64,
    uuid::Uuid,
    serde_json::Value,
    Vec<String>,
    Vec<uuid::Uuid>,
    Vec<i32>,
    Vec<i64>,
    Vec<f64>,
    Vec<u8>,
);

impl<T: serde::de::DeserializeOwned + Send + 'static> Column for sqlx::types::Json<T> {
    type Db = sqlx::types::Json<T>;
    fn from_db(v: Self::Db) -> Self {
        v
    }
}

impl Column for jiff::Timestamp {
    type Db = jiff_sqlx::Timestamp;
    fn from_db(v: Self::Db) -> Self {
        v.to_jiff()
    }
}

impl Column for jiff::civil::Date {
    type Db = jiff_sqlx::Date;
    fn from_db(v: Self::Db) -> Self {
        v.to_jiff()
    }
}

impl Column for jiff::civil::Time {
    type Db = jiff_sqlx::Time;
    fn from_db(v: Self::Db) -> Self {
        v.to_jiff()
    }
}

impl<T: Column> Column for Option<T>
where
    Option<T::Db>: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    type Db = Option<T::Db>;
    fn from_db(v: Self::Db) -> Self {
        v.map(T::from_db)
    }
}

pub trait RowExt {
    /// The named column as `T`; the error names the column.
    fn col<T: Column>(&self, name: &str) -> Result<T>;
}

impl RowExt for Row {
    fn col<T: Column>(&self, name: &str) -> Result<T> {
        self.0
            .try_get::<T::Db, _>(name)
            .map(T::from_db)
            .map_err(|e| Error::Database(sqlx::Error::Protocol(format!("column {name}: {e}"))))
    }
}

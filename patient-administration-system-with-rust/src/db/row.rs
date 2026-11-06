//! Reading columns out of a row (QUAL-7).
//!
//! `row.col::<T>("name")?` says which column failed. The error is a database error (so it is withheld from callers
//! like any other, see `Error::client_message`) and names the column in the log.

use sqlx::{Postgres, Row as _};

use crate::db::sql::Row;
use crate::{Error, Result};

/// A type a column can be read as.
pub trait Column: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres> {}

impl<T: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>> Column for T {}

pub trait RowExt {
    /// The named column as `T`; the error names the column.
    fn col<T: Column>(&self, name: &str) -> Result<T>;
}

impl RowExt for Row {
    fn col<T: Column>(&self, name: &str) -> Result<T> {
        self.0
            .try_get::<T, _>(name)
            .map_err(|e| Error::Database(sqlx::Error::Protocol(format!("column {name}: {e}"))))
    }
}

//! Reading columns out of a row (QUAL-7).
//!
//! `row.col::<T>("name")?` says which column failed. The error is a database error (so it is withheld from callers
//! like any other, see `Error::client_message`) and names the column in the log.

use sea_orm::{DbErr, TryGetable};

use crate::db::sql::Row;
use crate::{Error, Result};

pub trait RowExt {
    /// The named column as `T`; the error names the column.
    fn col<T: TryGetable>(&self, name: &str) -> Result<T>;
}

impl RowExt for Row {
    fn col<T: TryGetable>(&self, name: &str) -> Result<T> {
        self.0
            .try_get::<T>("", name)
            .map_err(|e| Error::Database(DbErr::Custom(format!("column {name}: {e}"))))
    }
}

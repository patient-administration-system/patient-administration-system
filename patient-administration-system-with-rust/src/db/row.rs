//! Reading columns out of a raw-SQL row (QUAL-7).
//!
//! `row.col::<T>("name")?` is `try_get("", "name")` that says which column failed. The error is a database
//! error (so it is withheld from callers like any other, see `Error::client_message`) and names the column in
//! the log, where the bare `try_get` error did not.

use sea_orm::{DbErr, QueryResult, TryGetable};

use crate::{Error, Result};

pub trait RowExt {
    /// The named column as `T`; the error names the column.
    fn col<T: TryGetable>(&self, name: &str) -> Result<T>;
}

impl RowExt for QueryResult {
    fn col<T: TryGetable>(&self, name: &str) -> Result<T> {
        self.try_get::<T>("", name)
            .map_err(|e| Error::Database(DbErr::Custom(format!("column {name}: {e}"))))
    }
}

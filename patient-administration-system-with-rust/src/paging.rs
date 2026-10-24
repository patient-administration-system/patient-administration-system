//! Lists that say when they were cut short (QUAL-14).
//!
//! A list endpoint takes `?limit=` (1 to [`MAX_LIMIT`]); a value outside that is a `400`, not silently
//! changed. It asks its service for one row more than the limit, returns at most `limit` rows, and says in
//! the response's `meta` whether there were more (`truncated`). A work queue that stops at 200 rows without
//! saying so loses the 201st person's call.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The most rows one list request may ask for.
pub const MAX_LIMIT: i64 = 500;

/// What a list response says about itself, under `meta` in the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PageMeta {
    /// The limit that applied (the one asked for, or the endpoint's default).
    pub limit: i64,
    /// How many rows are in `data`.
    pub returned: usize,
    /// `true` when more rows exist than were returned: ask again with a narrower question or a larger limit.
    pub truncated: bool,
}

/// The limit for a request: the one asked for, or `default`. Refused when outside 1 to [`MAX_LIMIT`].
pub fn limit(requested: Option<i64>, default: i64) -> Result<i64> {
    let n = requested.unwrap_or(default);
    if (1..=MAX_LIMIT).contains(&n) {
        Ok(n)
    } else {
        Err(Error::validation(format!(
            "limit must be between 1 and {MAX_LIMIT}"
        )))
    }
}

/// What a service should be asked for, given the request's limit: one more row, to learn whether there is more.
/// Services use [`fetch`] on whatever they are given so that a caller passing `limit + 1` is not cut back.
pub fn ask_for(limit: i64) -> i64 {
    limit + 1
}

/// Clamp the number of rows a service query returns: 1 to [`MAX_LIMIT`] + 1 (the extra row is how a handler
/// finds out the list was cut short).
pub fn fetch(limit: i64) -> i64 {
    limit.clamp(1, MAX_LIMIT + 1)
}

/// Keep at most `limit` rows and report whether there were more.
pub fn cut<T>(mut rows: Vec<T>, limit: i64) -> (Vec<T>, PageMeta) {
    let limit_rows = usize::try_from(limit).unwrap_or(0);
    let truncated = rows.len() > limit_rows;
    rows.truncate(limit_rows);
    let meta = PageMeta {
        limit,
        returned: rows.len(),
        truncated,
    };
    (rows, meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_outside_the_range_is_refused_not_changed() {
        assert_eq!(limit(None, 100).unwrap(), 100);
        assert_eq!(limit(Some(1), 100).unwrap(), 1);
        assert_eq!(limit(Some(MAX_LIMIT), 100).unwrap(), MAX_LIMIT);
        for bad in [0, -1, MAX_LIMIT + 1, i64::MAX, i64::MIN] {
            assert!(limit(Some(bad), 100).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_list_longer_than_its_limit_is_cut_and_says_so() {
        let (rows, meta) = cut(vec![1, 2, 3, 4], 3);
        assert_eq!(rows, vec![1, 2, 3]);
        assert_eq!(
            meta,
            PageMeta {
                limit: 3,
                returned: 3,
                truncated: true
            }
        );
        let (rows, meta) = cut(vec![1, 2, 3], 3);
        assert_eq!(
            (rows.len(), meta.truncated),
            (3, false),
            "exactly the limit is not truncated"
        );
        let (rows, meta) = cut(Vec::<i32>::new(), 3);
        assert_eq!((rows.len(), meta.truncated), (0, false));
    }

    #[test]
    fn the_service_side_allows_the_one_extra_row() {
        assert_eq!(fetch(ask_for(MAX_LIMIT)), MAX_LIMIT + 1);
        assert_eq!(fetch(0), 1);
        assert_eq!(fetch(i64::MAX), MAX_LIMIT + 1);
    }
}

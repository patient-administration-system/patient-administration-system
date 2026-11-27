//! Lists that say when they were cut short (QUAL-14), and signed cursors to the rest of them.
//!
//! A list endpoint takes `?limit=` (1 to [`MAX_LIMIT`]) and an opaque `?cursor=`. A limit outside that range,
//! or a cursor this server did not sign for this path and these filters, is a `422`, never changed or
//! ignored. The handler asks its service for one row more than the limit, returns at most `limit` rows, and
//! says in `meta` whether there were more (`truncated`). When there are, `meta.next` is the path and query of
//! the next page, and the response carries the same as `Link: <...>; rel="next"`. There is no total.
//!
//! A cursor is signed with HMAC-SHA256 under `PAS_CURSOR_KEY` (`axa-cursor-pagination`), over its position
//! and the query it belongs to, so a caller cannot make one up, edit one, or carry it to another list or
//! filter; it may change `limit` between pages. Every replica must share the key for a cursor from one to
//! work on another; with none set each process makes its own, and a cursor lasts only as long as it does.
//!
//! The position is an offset, so a row added or removed between two pages can make a later page skip or
//! repeat one row. Every paged query orders by something that ends in a unique column, so that the order is
//! at least stable. A work queue that stops at 200 rows without saying so loses the 201st person's call.

use std::sync::OnceLock;

use axa_cursor_pagination::{CursorKey, CursorLimits, CursorPage, MIN_KEY_LEN};
use axum::http::Uri;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};
use patient_administration_system_domain::arith::in_range;

/// The most rows one list request may ask for.
pub const MAX_LIMIT: i64 = 500;

/// What a list response says about itself, under `meta` in the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PageMeta {
    /// The limit that applied (the one asked for, or the endpoint's default).
    pub limit: i64,
    /// How many rows are in `data`.
    pub returned: usize,
    /// `true` when more rows exist than were returned: follow `next`, or ask a narrower question.
    pub truncated: bool,
    /// The path and query of the next page, with its signed `cursor`, when there is one. The same is in the
    /// response's `Link: rel="next"` header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// The `?limit=` and `?cursor=` every paged list takes, for the OpenAPI document.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// Rows to return, 1 to 500. Each list has its own default.
    pub limit: Option<i64>,
    /// The next page, from `meta.next` or the `Link` header of the page before. Opaque, and tied to this
    /// list and its filters.
    pub cursor: Option<String>,
}

fn bad_limit() -> Error {
    Error::Unprocessable(format!(
        "limit must be a whole number from 1 to {MAX_LIMIT}"
    ))
}

/// The limit for a list that has no next page: the `?limit=` asked for, as text, or `default`. Refused when
/// it is not a whole number from 1 to [`MAX_LIMIT`], exactly as [`Page::new`] refuses it, so that `limit=abc`
/// is the same `422` on every list.
pub fn limit(requested: Option<&str>, default: i64) -> Result<i64> {
    let Some(text) = requested else {
        return Ok(in_range("limit", default, 1, MAX_LIMIT)?);
    };
    text.parse::<i64>()
        .ok()
        .filter(|n| (1..=MAX_LIMIT).contains(n))
        .ok_or_else(bad_limit)
}

/// What a service should be asked for, given the request's limit: one more row, to learn whether there is more.
/// Services use [`fetch`] on whatever they are given so that a caller passing `limit + 1` is not cut back.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: `limit` is a page limit, checked to be at most MAX_LIMIT before it is used, so one more cannot overflow an i64"
)]
pub fn ask_for(limit: i64) -> i64 {
    limit + 1
}

/// The number of rows a service query returns: 1 to [`MAX_LIMIT`] + 1 (the extra row is how a handler finds
/// out the list was cut short). A number outside that is refused, never moved to the nearest bound (RCA-IN-2).
pub fn fetch(limit: i64) -> Result<i64> {
    Ok(in_range("limit", limit, 1, MAX_LIMIT + 1)?)
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
        next: None,
    };
    (rows, meta)
}

/// What changed in a window (TSC-6), for an integrator that polls rather than subscribes: `updated_since`
/// (inclusive) and, optionally, `updated_before` (exclusive). A list given `updated_since` lists the rows whose
/// `updated_at` falls in the window, oldest change first (ties by id), and its cursor holds the window.
///
/// Walk with a fixed `updated_before`: the time the walk started, which is then the next poll's `updated_since`.
/// Without it a row changed during the walk moves to the end of the list, and the rows after its old place each
/// move up one, so one of them can fall between two pages. A transaction that commits after the walk read its
/// window can still be missed; ending `updated_before` a few seconds in the past makes that unlikely.
#[derive(Debug, Clone, Copy, Default, Deserialize, utoipa::IntoParams)]
pub struct ChangedQuery {
    /// Rows changed at or after this time (RFC 3339).
    pub updated_since: Option<jiff::Timestamp>,
    /// Rows changed before this time (RFC 3339); only with `updated_since`.
    pub updated_before: Option<jiff::Timestamp>,
}

impl ChangedQuery {
    /// The window, or `None` for a list read as before. `updated_before` alone, or not after `updated_since`,
    /// is refused.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a window that cannot be walked.
    pub fn window(&self) -> Result<Option<(jiff::Timestamp, Option<jiff::Timestamp>)>> {
        match (self.updated_since, self.updated_before) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(Error::validation(
                "updated_before is used with updated_since",
            )),
            (Some(since), Some(before)) if before <= since => Err(Error::validation(
                "updated_before must be after updated_since",
            )),
            (Some(since), before) => Ok(Some((since, before))),
        }
    }
}

/// The rows a service query reads: `LIMIT` and `OFFSET`. A plain number converts to the first page, so a
/// caller that does not page passes its limit as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows {
    limit: i64,
    offset: i64,
}

impl Rows {
    /// `limit` rows from `offset`, for a caller that reads a whole list a page at a time, such as a test that
    /// must find its own row however long the list has grown.
    pub fn window(limit: i64, offset: i64) -> Self {
        Self { limit, offset }
    }

    /// `LIMIT`: the rows asked for, which [`fetch`] refuses outside 1 to [`MAX_LIMIT`] + 1.
    pub fn fetch(self) -> Result<i64> {
        fetch(self.limit)
    }

    /// `LIMIT` as given, for a repository whose callers include more than list routes: not negative.
    pub fn limit(self) -> Result<i64> {
        Ok(in_range("limit", self.limit, 0, i64::MAX)?)
    }

    /// `OFFSET`: rows to skip, not negative.
    pub fn offset(self) -> Result<i64> {
        Ok(in_range("offset", self.offset, 0, i64::MAX)?)
    }
}

impl From<i64> for Rows {
    fn from(limit: i64) -> Self {
        Self { limit, offset: 0 }
    }
}

impl From<i32> for Rows {
    fn from(limit: i32) -> Self {
        Self::from(i64::from(limit))
    }
}

impl From<u64> for Rows {
    fn from(limit: u64) -> Self {
        Self::from(i64::try_from(limit).unwrap_or(i64::MAX))
    }
}

/// The path and query inside `<...>; rel="next"`.
fn link_target(link: &axum::http::HeaderValue) -> Option<String> {
    let link = link.to_str().ok()?;
    let (_, after) = link.split_once('<')?;
    let (url, _) = after.split_once('>')?;
    Some(url.to_string())
}

static KEY: OnceLock<CursorKey> = OnceLock::new();

/// A key for this process alone, for when `PAS_CURSOR_KEY` is not set: 244 random bits from two v4 UUIDs.
fn process_key() -> Result<CursorKey> {
    let mut secret = [0u8; 32];
    secret[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    secret[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    CursorKey::new(&secret).map_err(|_| Error::internal("a cursor key of 32 bytes was refused"))
}

/// Set the key that signs cursors, from `PAS_CURSOR_KEY`. `main` calls this once at start-up; without it, or
/// with the variable unset, each process signs with a key of its own.
///
/// # Errors
///
/// A `PAS_CURSOR_KEY` shorter than 32 bytes is a configuration error, so that a weak key stops start-up.
pub fn init_cursor_key(secret: Option<&str>) -> Result<bool> {
    let key = match secret {
        Some(secret) => CursorKey::new(secret.as_bytes()).map_err(|_| {
            Error::config(format!(
                "PAS_CURSOR_KEY must be at least {MIN_KEY_LEN} bytes"
            ))
        })?,
        None => process_key()?,
    };
    let _ = KEY.set(key);
    Ok(secret.is_some())
}

fn key() -> Result<&'static CursorKey> {
    if let Some(k) = KEY.get() {
        return Ok(k);
    }
    let made = process_key()?;
    Ok(KEY.get_or_init(|| made))
}

/// One page of a list, read from the request's URI: its limit, and where its signed cursor starts.
#[derive(Debug, Clone)]
pub struct Page {
    inner: CursorPage,
}

impl Page {
    /// Read `?limit=` (or `default_limit`) and `?cursor=` from `uri`, the request's original URI.
    ///
    /// # Errors
    ///
    /// [`Error::Unprocessable`] for a limit outside 1 to [`MAX_LIMIT`], or a cursor this server did not sign
    /// for this path and these filters. Neither quotes what was sent.
    pub fn new(uri: &Uri, default_limit: i64) -> Result<Self> {
        // A list's own default is a figure in the code, so one outside 1 to MAX_LIMIT is a defect to find at once.
        let default_limit = in_range("default limit", default_limit, 1, MAX_LIMIT)?;
        let limits = CursorLimits {
            default_limit: u64::try_from(default_limit)
                .map_err(|_| Error::internal("default limit is not a count"))?,
            max_limit: u64::try_from(MAX_LIMIT)
                .map_err(|_| Error::internal("the page maximum is not a count"))?,
        };
        CursorPage::new(key()?, limits, uri)
            .map(|inner| Self { inner })
            .map_err(|e| match e {
                axa_cursor_pagination::PageError::BadLimit { .. } => bad_limit(),
                _ => Error::Unprocessable("cursor is not valid for this list".into()),
            })
    }

    /// The limit that applies.
    pub fn limit(&self) -> i64 {
        i64::try_from(self.inner.limit()).unwrap_or(MAX_LIMIT)
    }

    /// What to ask the service for: one row more than the limit, from the cursor's position.
    pub fn rows(&self) -> Rows {
        let (limit, offset) = self.inner.bounds();
        Rows { limit, offset }
    }

    /// The `Link` header for the next page.
    pub fn link(&self) -> Option<axum::http::HeaderValue> {
        self.inner.next_link()
    }

    /// Fill this page from `fetch`, keeping only the rows `keep` accepts, for a list whose rows are kept or
    /// not by something worked out in Rust rather than in SQL (`axa-cursor-pagination` 0.1.4).
    ///
    /// `fetch(rows)` reads a window in the list's order (which must end in a unique column); it is asked for a
    /// page and one row more at a time until the page is full, the list ends, or `max_scan` rows have been read.
    /// The next page starts right after the last row returned, so nothing is skipped or shown twice. A page that
    /// spent its budget may be short and still have a next page. Returns the rows, `meta` as [`Page::cut`] gives
    /// it, and the `Link` header for the next page.
    ///
    /// # Errors
    ///
    /// The first error `fetch` returns.
    pub async fn fill<T, F, Fut, K>(
        &self,
        max_scan: u64,
        mut fetch: F,
        keep: K,
    ) -> Result<(Vec<T>, PageMeta, Option<axum::http::HeaderValue>)>
    where
        F: FnMut(Rows) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<T>>>,
        K: FnMut(&T) -> bool,
    {
        let filled = self
            .inner
            .fill(
                max_scan,
                |limit, offset| fetch(Rows { limit, offset }),
                keep,
            )
            .await?;
        let meta = PageMeta {
            limit: self.limit(),
            returned: filled.rows.len(),
            truncated: filled.next.is_some(),
            next: filled.next.as_ref().and_then(link_target),
        };
        Ok((filled.rows, meta, filled.next))
    }

    /// Keep at most the limit, and say whether there were more and where the next page is.
    pub fn cut<T>(&self, rows: Vec<T>) -> (Vec<T>, PageMeta) {
        let (rows, truncated) = self.inner.split(rows);
        let next = truncated
            .then(|| self.link())
            .flatten()
            .as_ref()
            .and_then(link_target);
        let meta = PageMeta {
            limit: self.limit(),
            returned: rows.len(),
            truncated,
            next,
        };
        (rows, meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn a_limit_outside_the_range_is_refused_not_changed() {
        assert_eq!(limit(None, 100).unwrap(), 100);
        assert_eq!(limit(Some("1"), 100).unwrap(), 1);
        assert_eq!(limit(Some("500"), 100).unwrap(), MAX_LIMIT);
        for bad in ["0", "-1", "501", "99999999999999999999", "abc", ""] {
            assert!(
                matches!(limit(Some(bad), 100), Err(Error::Unprocessable(_))),
                "{bad}"
            );
        }
        for bad in ["0", "-1", "501", "abc", "1&limit=2"] {
            let page = Page::new(&uri(&format!("/api/x?limit={bad}")), 100);
            assert!(matches!(page, Err(Error::Unprocessable(_))), "{bad}");
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
                truncated: true,
                next: None,
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
        assert_eq!(fetch(ask_for(MAX_LIMIT)).unwrap(), MAX_LIMIT + 1);
        assert_eq!(fetch(1).unwrap(), 1);
        // Outside the range is refused, not moved to the nearest bound.
        for bad in [0, -1, MAX_LIMIT + 2, i64::MAX, i64::MIN] {
            let e = fetch(bad).unwrap_err().to_string();
            assert!(e.contains("limit") && e.contains("1 to 501"), "{bad}: {e}");
        }
        assert_eq!(Rows::from(7_i64).offset().unwrap(), 0);
        assert!(Rows::window(10, -1).offset().is_err());
        assert!(Rows::window(-1, 0).limit().is_err());
        assert!(
            Page::new(&uri("/api/x"), 0).is_err(),
            "a default of 0 is a defect"
        );
        assert!(Page::new(&uri("/api/x"), MAX_LIMIT + 1).is_err());
    }

    #[test]
    fn a_cut_page_points_to_the_next_and_the_next_starts_after_it() {
        let first = Page::new(&uri("/api/things?status=open&limit=2"), 100).unwrap();
        assert_eq!(
            (
                first.rows().fetch().unwrap(),
                first.rows().offset().unwrap()
            ),
            (3, 0)
        );
        let (rows, meta) = first.cut(vec!["a", "b", "c"]);
        assert_eq!(rows, vec!["a", "b"]);
        let next = meta.next.expect("a next page");
        assert!(
            next.starts_with("/api/things?status=open&limit=2&cursor="),
            "{next}"
        );

        let second = Page::new(&uri(&next), 100).unwrap();
        assert_eq!(
            (
                second.rows().fetch().unwrap(),
                second.rows().offset().unwrap()
            ),
            (3, 2)
        );
        let (_, meta) = second.cut(vec!["c"]);
        assert_eq!((meta.truncated, meta.next), (false, None));
    }

    #[test]
    fn a_cursor_works_only_on_its_own_list_and_filters() {
        let first = Page::new(&uri("/api/things?status=open&limit=1"), 100).unwrap();
        let next = first.cut(vec![1, 2]).1.next.unwrap();
        let cursor = next.split("cursor=").nth(1).unwrap();
        for other in [
            format!("/api/other?status=open&cursor={cursor}"),
            format!("/api/things?status=closed&cursor={cursor}"),
            format!("/api/things?cursor={cursor}"),
            "/api/things?status=open&cursor=made-up".to_string(),
        ] {
            assert!(
                matches!(Page::new(&uri(&other), 100), Err(Error::Unprocessable(_))),
                "{other}"
            );
        }
        let wider = format!("/api/things?status=open&limit=50&cursor={cursor}");
        assert_eq!(
            Page::new(&uri(&wider), 100)
                .unwrap()
                .rows()
                .offset()
                .unwrap(),
            1
        );
    }

    /// A page of a list whose rows are kept in Rust (axa-cursor-pagination 0.1.4 `fill`): full pages, each
    /// row once across pages, and a read budget that stops a request early with a next page from where it
    /// stopped.
    #[tokio::test]
    async fn a_filled_page_is_full_reaches_every_kept_row_once_and_keeps_to_its_budget() {
        let table: Vec<i64> = (0..20).collect();
        let fetch = |rows: Rows| {
            let table = table.clone();
            async move {
                let from = usize::try_from(rows.offset()?)
                    .unwrap_or(usize::MAX)
                    .min(table.len());
                let to = from
                    .saturating_add(usize::try_from(rows.limit()?).unwrap_or(0))
                    .min(table.len());
                Ok(table[from..to].to_vec())
            }
        };
        let even = |n: &i64| n % 2 == 0;
        let mut seen = Vec::new();
        let mut at = "/api/x?limit=3".to_string();
        loop {
            let page = Page::new(&uri(&at), 100).unwrap();
            let (rows, meta, link) = page.fill(1_000, fetch, even).await.unwrap();
            assert_eq!(meta.returned, rows.len());
            assert_eq!(meta.truncated, link.is_some());
            if meta.truncated {
                assert_eq!(rows.len(), 3, "a page with more after it is full");
            }
            seen.extend(rows);
            match meta.next {
                Some(next) => at = next,
                None => break,
            }
        }
        assert_eq!(seen, (0..20).filter(|n| n % 2 == 0).collect::<Vec<_>>());

        // Nothing kept for the first sixteen rows and a budget of eight: the page stops early, short, with a
        // next page that carries on from where the reading stopped.
        let late = |n: &i64| *n >= 16;
        let page = Page::new(&uri("/api/x?limit=3"), 100).unwrap();
        let (rows, meta, _) = page.fill(8, fetch, late).await.unwrap();
        assert!(rows.is_empty() && meta.truncated, "{meta:?}");
        let next = Page::new(&uri(&meta.next.unwrap()), 100).unwrap();
        assert_eq!(next.rows().offset().unwrap(), 8);
    }

    #[test]
    fn a_short_key_is_refused() {
        assert!(matches!(
            init_cursor_key(Some("too short")),
            Err(Error::Config(_))
        ));
    }
}

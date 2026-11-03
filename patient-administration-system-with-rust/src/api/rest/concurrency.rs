//! Optimistic concurrency (OFH-1, NFR-27): the version a caller read, and the check that it is still current.
//!
//! A record's version is its `updated_at`, to the microsecond (the precision the database keeps). A single
//! record in a response from `/api/appointments/`, `/api/waitlist/` or `/api/admissions/` carries it as
//! `ETag: "<updated_at>"`. A write sends it back as `If-Match`:
//!
//! - absent: `428 PRECONDITION_REQUIRED`, nothing changed;
//! - `*`: any version (a caller that does not care says so);
//! - a version that is no longer current: `412 PRECONDITION_FAILED`, nothing changed;
//! - not a time: `400`.

use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, header},
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{Error, Result};

/// The tag for a version: the time to the microsecond, in quotes.
pub fn etag(updated_at: DateTime<Utc>) -> String {
    format!(
        "\"{}\"",
        updated_at.to_rfc3339_opts(SecondsFormat::Micros, true)
    )
}

/// The version named in `If-Match`. `Ok(None)` means the caller said `*` (any version).
/// A missing header is `428`; a value that is not a time is `400`.
pub fn require_if_match(headers: &HeaderMap, what: &str) -> Result<Option<DateTime<Utc>>> {
    let Some(raw) = headers.get(header::IF_MATCH) else {
        return Err(Error::precondition_required(format!(
            "send If-Match with the {what}'s ETag (its updated_at) to change it; read it first, or send * to overwrite"
        )));
    };
    let text = raw
        .to_str()
        .map_err(|_| Error::validation("If-Match is not text"))?
        .trim();
    if text == "*" {
        return Ok(None);
    }
    let text = text.strip_prefix("W/").unwrap_or(text).trim_matches('"');
    DateTime::parse_from_rfc3339(text)
        .map(|t| Some(t.with_timezone(&Utc)))
        .map_err(|_| Error::validation(format!("If-Match must be the {what}'s ETag")))
}

/// Is the stored version the one the caller read? `None` (sent `*`) always is.
pub fn is_current(stored: DateTime<Utc>, seen: Option<DateTime<Utc>>) -> bool {
    seen.is_none_or(|s| s.timestamp_micros() == stored.timestamp_micros())
}

/// The id a versioned path names (`/api/appointments/{id}...`), if it is one.
fn versioned_id(path: &str) -> Option<&str> {
    [
        "/api/appointments/",
        "/api/waitlist/",
        "/api/admissions/",
        "/api/encounters/",
        "/api/beds/",
    ]
    .iter()
    .find_map(|p| path.strip_prefix(p))
    .and_then(|rest| rest.split('/').next())
    .filter(|id| !id.is_empty())
}

/// Adds `ETag` to a successful JSON response that is the record the path names: `data` an object whose `id`
/// is the id in the path and which has a string `updated_at`. A response about something else (a transfer, a
/// new appointment from a rebook, a list) passes through untouched, so a tag is never the version of a
/// different record than the one the client asked about.
pub async fn etag_middleware(req: Request, next: Next) -> Response {
    let named = versioned_id(req.uri().path()).map(str::to_owned);
    let wanted = named.is_some()
        && matches!(
            *req.method(),
            Method::GET | Method::POST | Method::PUT | Method::PATCH
        );
    let res = next.run(req).await;
    if !wanted || !res.status().is_success() {
        return res;
    }
    let is_json = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = to_bytes(body, 4 * 1024 * 1024).await else {
        return Response::from_parts(parts, Body::empty());
    };
    if let Some(tag) = version_of(&bytes, named.as_deref().unwrap_or_default())
        .and_then(|t| HeaderValue::from_str(&etag(t)).ok())
    {
        parts.headers.insert(header::ETAG, tag);
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(bytes))
}

/// The `updated_at` of the record in an envelope, if its `id` is `id`.
fn version_of(bytes: &[u8], id: &str) -> Option<DateTime<Utc>> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let data = v.get("data")?.as_object()?;
    (data.get("id")?.as_str()? == id).then_some(())?;
    let at = data.get("updated_at")?.as_str()?;
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(v: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = v {
            h.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn a_missing_if_match_is_428_star_is_any_and_garbage_is_400() {
        assert!(matches!(
            require_if_match(&headers(None), "appointment"),
            Err(Error::PreconditionRequired(_))
        ));
        assert_eq!(require_if_match(&headers(Some("*")), "x").unwrap(), None);
        assert!(matches!(
            require_if_match(&headers(Some("\"nonsense\"")), "x"),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn the_tag_round_trips_and_compares_to_the_microsecond() {
        let t = DateTime::parse_from_rfc3339("2026-11-02T10:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let seen = require_if_match(&headers(Some(&etag(t))), "x").unwrap();
        assert!(is_current(t, seen));
        assert!(is_current(t, None));
        assert!(!is_current(t + chrono::Duration::microseconds(1), seen));
        let weak = format!("W/{}", etag(t));
        assert!(is_current(
            t,
            require_if_match(&headers(Some(&weak)), "x").unwrap()
        ));
    }

    #[test]
    fn only_the_record_the_path_names_has_a_version() {
        let one = br#"{"success":true,"data":{"id":"a","updated_at":"2026-11-02T10:00:00.5Z"}}"#;
        assert!(version_of(one, "a").is_some());
        assert!(
            version_of(one, "b").is_none(),
            "a record the path does not name has no tag"
        );
        assert!(
            version_of(
                br#"{"success":true,"data":[{"id":"a","updated_at":"2026-11-02T10:00:00Z"}]}"#,
                "a"
            )
            .is_none()
        );
        assert!(version_of(br#"{"success":true,"data":{"id":"a"}}"#, "a").is_none());
    }

    #[test]
    fn the_versioned_paths_are_appointments_waitlist_and_admissions() {
        assert_eq!(versioned_id("/api/appointments/abc"), Some("abc"));
        assert_eq!(versioned_id("/api/waitlist/abc/book"), Some("abc"));
        assert_eq!(versioned_id("/api/admissions/abc/discharge"), Some("abc"));
        assert_eq!(versioned_id("/api/encounters/abc/status"), Some("abc"));
        assert_eq!(versioned_id("/api/beds/abc/status"), Some("abc"));
        assert_eq!(versioned_id("/api/patients/abc"), None);
        assert_eq!(versioned_id("/api/admissions/"), None);
    }
}

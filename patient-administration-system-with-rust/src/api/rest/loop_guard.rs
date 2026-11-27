//! A write that repeats too often (TSC-12): the same caller sending the same change to the same record again
//! and again within a short window, as a write-back loop between our webhook and a subscriber's API call does.
//!
//! Each POST, PUT, PATCH and DELETE is counted by a hash of its caller (`x-user-id`, set by the authentication
//! layer), method, path with its query, and body. Past `PAS_LOOP_LIMIT` of the same in `PAS_LOOP_WINDOW_SECONDS`
//! (default 10 in 60; a limit of 0 turns this off) it is refused with `429`, `code: repeated_change` and
//! `Retry-After`, and logged once for that window, so the loop is seen. A different change, another record or
//! another caller is counted apart. Reads are never counted.
//!
//! The counts are in this process: with several replicas behind a balancer a loop is caught later, at the
//! limit on each replica it reaches. They hold only hashes, and an entry older than its window is dropped.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

/// A request body this reads to count; a larger one is not counted (it is passed on as it was).
const MAX_BODY: usize = 4 * 1024 * 1024;
/// Past this many entries, those whose window has ended are dropped.
const PRUNE_AT: usize = 10_000;

#[derive(Debug, Clone, Copy)]
struct Seen {
    first: Instant,
    count: u32,
    logged: bool,
}

fn counts() -> &'static Mutex<HashMap<[u8; 32], Seen>> {
    static COUNTS: OnceLock<Mutex<HashMap<[u8; 32], Seen>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How long since `then`, or zero when `then` is not before `now` (a clock read out of order).
fn since(now: Instant, then: Instant) -> Duration {
    now.checked_duration_since(then).unwrap_or(Duration::ZERO)
}

/// Whether a request with this `key` is refused at `now`: `Some((wait, first_refusal))` once `limit` of the
/// same have been done in the window, `None` otherwise. An entry whose window has ended starts afresh.
fn refused(key: [u8; 32], limit: u32, window: Duration, now: Instant) -> Option<(Duration, bool)> {
    let mut map = counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if map.len() > PRUNE_AT {
        map.retain(|_, s| since(now, s.first) < window);
    }
    let seen = map.get_mut(&key)?;
    if since(now, seen.first) >= window {
        map.remove(&key);
        return None;
    }
    if seen.count < limit {
        return None;
    }
    let first_refusal = !seen.logged;
    seen.logged = true;
    let wait = window
        .checked_sub(since(now, seen.first))
        .unwrap_or(Duration::ZERO);
    Some((wait, first_refusal))
}

/// A request with this `key` was done (its answer was a success) at `now`.
fn record(key: [u8; 32], window: Duration, now: Instant) {
    let mut map = counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let seen = map.entry(key).or_insert(Seen {
        first: now,
        count: 0,
        logged: false,
    });
    if since(now, seen.first) >= window {
        *seen = Seen {
            first: now,
            count: 0,
            logged: false,
        };
    }
    // A count at the maximum stays there: it is only compared with the limit.
    seen.count = seen.count.checked_add(1).unwrap_or(u32::MAX);
}

/// The settings, read on each request so a change takes effect without a restart: `(limit, window)`, or
/// `None` when the guard is off.
fn settings() -> Option<(u32, Duration)> {
    // Both are in the settings registry, so a value that does not parse stops the server at start; one that
    // stops parsing later (changed in a running process) is reported and the guard stands aside.
    let read = || -> crate::Result<(u32, u64)> {
        Ok((
            crate::config::env::count::<u32>("PAS_LOOP_LIMIT")?,
            crate::config::env::count::<u64>("PAS_LOOP_WINDOW_SECONDS")?,
        ))
    };
    match read() {
        Ok((limit, window)) => {
            (limit > 0 && window > 0).then(|| (limit, Duration::from_secs(window)))
        }
        Err(e) => {
            tracing::error!(target: "pas::loop_guard", "the loop guard's settings cannot be read: {e}");
            None
        }
    }
}

pub async fn loop_guard_middleware(req: Request, next: Next) -> Response {
    let writes = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let Some((limit, window)) = settings().filter(|_| writes) else {
        return next.run(req).await;
    };
    // The caller's raw bytes: no header is the anonymous caller, and one that is not text is still told apart.
    let caller: Vec<u8> = match req.headers().get("x-user-id") {
        Some(v) => v.as_bytes().to_vec(),
        None => Vec::new(),
    };
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        // Too large to read here; the handler's own limit answers it.
        return crate::api::rest::handlers::error_to_response(crate::Error::Unprocessable(
            "the request body is too large".into(),
        ));
    };
    let mut hash = Sha256::new();
    for part in [
        caller.as_slice(),
        parts.method.as_str().as_bytes(),
        parts.uri.to_string().as_bytes(),
    ] {
        hash.update(part);
        hash.update([0]);
    }
    hash.update(&bytes);
    let key: [u8; 32] = hash.finalize().into();
    if let Some((wait, first_refusal)) = refused(key, limit, window, Instant::now()) {
        if first_refusal {
            // The route, not the path's values or the body: nothing here may carry personal data (PA-02).
            tracing::warn!(
                target: "pas::loop_guard",
                method = %parts.method,
                route = %crate::api::rest::loop_guard::route_of(parts.uri.path()),
                limit,
                window_secs = window.as_secs(),
                "the same change was sent too often; refused until the window ends"
            );
        }
        let mut response = crate::api::problem(
            StatusCode::TOO_MANY_REQUESTS,
            "repeated_change",
            format!(
                "the same change to the same record was sent more than {limit} times in {} seconds",
                window.as_secs()
            ),
        )
        .into_response();
        if let Ok(v) = HeaderValue::from_str(&wait.as_secs().max(1).to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return response;
    }
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    // Only a change that was made counts: a refused or failed request (a bad signature, a validation error) is
    // not a loop of changes, and counting it would lock out a caller fixing its own mistake.
    if response.status().is_success() {
        record(key, window, Instant::now());
    }
    response
}

/// A path with each value (anything not a plain route word) replaced by `{id}`, for a log line.
pub fn route_of(path: &str) -> String {
    path.split('/')
        .map(|s| {
            let word = !s.is_empty()
                && s.len() <= 32
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-' || c == '$' || c.is_ascii_digit())
                && !s.chars().next().is_some_and(|c| c.is_ascii_digit());
            if s.is_empty() || word {
                s.to_string()
            } else {
                "{id}".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_lets_that_many_through_then_refuses_until_the_window_ends() {
        let key = [7u8; 32];
        let start = Instant::now();
        let window = Duration::from_secs(60);
        for _ in 0..3 {
            assert!(refused(key, 3, window, start).is_none());
            record(key, window, start);
        }
        let (wait, first) = refused(key, 3, window, start + Duration::from_secs(10)).unwrap();
        assert!(first, "the first refusal is logged");
        assert_eq!(wait, Duration::from_secs(50));
        let (_, again) = refused(key, 3, window, start + Duration::from_secs(11)).unwrap();
        assert!(!again, "and only the first");
        assert!(
            refused(key, 3, window, start + Duration::from_secs(61)).is_none(),
            "a new window starts afresh"
        );
        assert!(
            refused([8u8; 32], 3, window, start).is_none(),
            "another request is counted apart"
        );
    }

    #[test]
    fn a_log_line_names_the_route_not_the_record() {
        assert_eq!(
            route_of(
                "/api/patients/6f9619ff-8b86-d011-b42d-00c04fc964ff/communication-needs/review"
            ),
            "/api/patients/{id}/communication-needs/review"
        );
        assert_eq!(
            route_of("/fhir/r4/Appointment/$book"),
            "/fhir/r4/{id}/$book"
        );
        assert_eq!(route_of("/api/hl7/v2/admit"), "/api/hl7/v2/admit");
    }
}

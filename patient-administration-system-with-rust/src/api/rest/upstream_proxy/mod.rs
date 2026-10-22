//! REST proxy handlers for the v0.44.0 Option C identity + workforce
//! delegation.
//!
//! Every `/api/patients/*` route here proxies to `person-service` via
//! [`crate::upstream::PersonClient`]. Every `/api/practitioners/*`
//! route proxies to `worker-service` via
//! [`crate::upstream::WorkerClient`]. Inbound merge webhooks land at
//! `/api/upstream/{person,worker}-merged` and propagate the upstream's
//! merge decision into PAS-owned FK columns.
//!
//! ## Wire-shape note
//!
//! This is a **transparent proxy**: the upstream's `Person` / `Worker` JSON shape is forwarded under the
//! PAS `ApiResponse` envelope verbatim. Callers see whatever `person-service` returns. This is a
//! deliberate trade-off of keeping identity upstream — see `spec.md` §4.0 wire-shape note.
//!
//! ## User context
//!
//! Every outbound upstream call attaches `X-User-Id` / `X-User-Ip` /
//! `X-User-Agent` headers extracted from the inbound request, so the
//! upstream's audit log credits the original PAS-side caller. Headers
//! that are absent from the inbound request are not attached.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{ok, ok_json};
use crate::upstream::{
    MergeOutcome, PersonClient, PersonCreate, UserContext, WorkerClient, WorkerCreate,
};

mod patients;
mod practitioners;
mod webhooks;
pub use patients::*;
pub use practitioners::*;
pub use webhooks::*;

// ─── User-context extractor ─────────────────────────────────────────────

/// Build an upstream [`UserContext`] from inbound `X-User-*` headers.
/// Every header is optional; missing headers leave the corresponding
/// field as `None`.
pub fn user_context_from_headers(headers: &HeaderMap) -> UserContext {
    let take = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string)
    };
    UserContext {
        user_id: take("x-user-id"),
        user_ip: take("x-user-ip"),
        user_agent: take("x-user-agent"),
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────────

fn need_person_client(state: &AppState) -> Result<&dyn PersonClient, Error> {
    state
        .person_client
        .as_deref()
        .ok_or_else(|| Error::upstream("PERSON_SERVICE_URL unset; identity proxy disabled"))
}

fn need_worker_client(state: &AppState) -> Result<&dyn WorkerClient, Error> {
    state
        .worker_client
        .as_deref()
        .ok_or_else(|| Error::upstream("WORKER_SERVICE_URL unset; workforce proxy disabled"))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

// ─── /api/patients/* (proxy person-service) ────────────────────────

// ─── /api/patients/export.{json,xml,tsv,csv} + /api/patients/import ────
//
// Bulk interchange. The PAS does not own patient rows, so
// every bulk export/import becomes a transparent passthrough to
// `person-service`. We do NOT wrap the body in the PAS `ApiResponse`
// envelope here — callers feeding the output into their own tooling
// (CSV → Excel, XML → integration engine, etc.) want the raw bytes
// with the upstream's content-type.

// ─── /api/practitioners/* (proxy worker-service) ───────────────────

// ─── Inbound merge webhooks ─────────────────────────────────────────────

// ─── Unit tests for the small helpers ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_user_context_extracts_present_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user-id", HeaderValue::from_static("alice"));
        headers.insert("x-user-ip", HeaderValue::from_static("10.0.0.1"));
        headers.insert("x-user-agent", HeaderValue::from_static("curl/8"));
        let ctx = user_context_from_headers(&headers);
        assert_eq!(ctx.user_id.as_deref(), Some("alice"));
        assert_eq!(ctx.user_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(ctx.user_agent.as_deref(), Some("curl/8"));
    }

    #[test]
    fn test_user_context_missing_headers_are_none() {
        let ctx = user_context_from_headers(&HeaderMap::new());
        assert!(ctx.user_id.is_none());
        assert!(ctx.user_ip.is_none());
        assert!(ctx.user_agent.is_none());
    }

    #[test]
    fn test_hex_decode_lowercase() {
        assert_eq!(hex_decode("abcd"), Some(vec![0xab, 0xcd]));
    }

    #[test]
    fn test_hex_decode_uppercase() {
        assert_eq!(hex_decode("ABCD"), Some(vec![0xab, 0xcd]));
    }

    #[test]
    fn test_hex_decode_odd_length_rejected() {
        assert_eq!(hex_decode("abc"), None);
    }

    #[test]
    fn test_hex_decode_non_hex_char_rejected() {
        assert_eq!(hex_decode("zzzz"), None);
    }

    #[test]
    fn test_verify_hmac_accepts_correct_signature() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let secret = b"my-secret";
        let body = b"hello world";
        let mut mac = <Hmac<Sha256>>::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(verify_hmac(secret, body, &sig).is_ok());
    }

    #[test]
    fn test_verify_hmac_rejects_wrong_secret() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let mut mac = <Hmac<Sha256>>::new_from_slice(b"other").unwrap();
        mac.update(b"hello world");
        let sig = format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(verify_hmac(b"my-secret", b"hello world", &sig).is_err());
    }

    #[test]
    fn test_verify_hmac_rejects_missing_prefix() {
        assert!(verify_hmac(b"k", b"v", "abcd").is_err());
    }
}

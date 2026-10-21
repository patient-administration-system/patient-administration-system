//! REST API handlers — **stubbed in v0.44.0 Phase 2**.
//!
//! Every previous REST handler in this module is replaced with a
//! `503 + UPSTREAM_UNAVAILABLE` stub for the duration of Phase 2. The
//! single exception is `/api/health`, which keeps its real body (the DB
//! ping aggregator gains upstream checks in Phase 3).
//!
//! Phase 3 reintroduces the typed handlers as either
//! (a) **proxies** over `crate::upstream::person_client` /
//! `crate::upstream::worker_client` (for every `/api/patients/*`,
//! `/api/practitioners/*`, and HL7 v2 PID-bearing route), or
//! (b) **direct PAS-owned reads** against the renamed `person_id` /
//! `worker_id` columns (for ADT / scheduling / waitlist / RTT /
//! billing / communication / consent / coverage).

use axum::{Json, extract::State, http::StatusCode};
use serde_json::Value;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::{ApiError, ApiResponse};

/// Standard handler return type: an HTTP status plus a JSON `ApiResponse`.
pub type HandlerResult = (StatusCode, Json<ApiResponse<Value>>);

/// Map a service-layer [`Error`] onto an HTTP status code + machine-readable
/// error code.
pub fn error_to_response(err: Error) -> HandlerResult {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        Error::Validation(_) => (StatusCode::BAD_REQUEST, "VALIDATION"),
        Error::InvalidStateTransition(_) => (StatusCode::CONFLICT, "INVALID_STATE_TRANSITION"),
        Error::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
        Error::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
        Error::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "DATABASE"),
        Error::Search(_) => (StatusCode::INTERNAL_SERVER_ERROR, "SEARCH"),
        Error::Streaming(_) => (StatusCode::INTERNAL_SERVER_ERROR, "STREAMING"),
        Error::Fhir(_) => (StatusCode::BAD_REQUEST, "FHIR"),
        Error::Render(_) => (StatusCode::INTERNAL_SERVER_ERROR, "RENDER"),
        Error::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "CONFIG"),
        Error::Upstream(_) => (StatusCode::SERVICE_UNAVAILABLE, "UPSTREAM_UNAVAILABLE"),
        Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    };
    (status, Json(ApiResponse::err(code, err.client_message())))
}

fn ok_response(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// Phase 2 catch-all: every non-health route returns 503 + UPSTREAM_UNAVAILABLE.
fn upstream_stub(what: &str) -> HandlerResult {
    error_to_response(Error::upstream(format!(
        "{what} handler is stubbed during v0.44.0 Phase 2; Phase 3 restores \
         it as a proxy over person-service / worker-service or as \
         a direct PAS-owned read against the renamed person_id / worker_id \
         columns."
    )))
}

/// `GET /api/health` — DB liveness probe. Phase 3 extends to include
/// `person_service` + `worker_service` aggregator.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn health(State(state): State<AppState>) -> HandlerResult {
    let db_ok = state.db.ping().await.map(|_| true).unwrap_or(false);
    let status = if db_ok { "ok" } else { "degraded" };
    let database = if db_ok { "ok" } else { "unreachable" };
    ok_response(serde_json::json!({
        "status": status,
        "database": database
    }))
}

/// Hand-written `ApiError` constructor wrapping a stub call. Kept for
/// completeness — nothing else in Phase 2 builds an [`ApiError`].
pub fn api_error_stub() -> ApiError {
    ApiError {
        code: "UPSTREAM_UNAVAILABLE".to_string(),
        message: "Phase 2 stub".to_string(),
    }
}

// ─── Catch-all stub handler ───────────────────────────────────────────
//
// Every non-health route in `routes.rs` is wired to this one function.
// Phase 3 replaces individual route bindings with their real handler
// signatures + bodies.
pub async fn stub_any() -> HandlerResult {
    upstream_stub("REST")
}

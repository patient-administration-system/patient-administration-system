//! Shared REST handler pieces: the `ApiResponse` helpers (`ok`, `ok_json`), the error mapping
//! (`error_to_response`), the audit-context helpers, and `GET /api/health`. The domain handlers live in the
//! `handlers_*` modules and `upstream_proxy`, wired up in `routes.rs`.

use axum::{Json, extract::State, http::StatusCode};
use serde_json::Value;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;

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

/// The `?limit=` of a list request (see [`crate::paging`]).
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

/// `200` with `value` in the `ApiResponse` envelope.
pub fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `200` with `value` serialized into the envelope. A value that cannot be serialized is a logged `500`,
/// never a silent `null` (QUAL-27).
pub fn ok_json<T: serde::Serialize>(value: T) -> HandlerResult {
    match serde_json::to_value(value) {
        Ok(v) => ok(v),
        Err(e) => error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        ))),
    }
}

/// `200` with a list: at most `limit` rows, and `meta` saying whether there were more (QUAL-14). Ask the
/// service for [`crate::paging::ask_for`]`(limit)` rows so there is one to tell the difference.
pub fn ok_page<T: serde::Serialize>(rows: Vec<T>, limit: i64) -> HandlerResult {
    let (rows, meta) = crate::paging::cut(rows, limit);
    match serde_json::to_value(rows) {
        Ok(v) => (StatusCode::OK, Json(ApiResponse::ok_page(v, meta))),
        Err(e) => error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        ))),
    }
}

/// The audit context of a request, from its headers.
pub fn request_audit_ctx(
    headers: &axum::http::HeaderMap,
) -> crate::db::repositories::audit::UserContext {
    audit_ctx(&crate::api::rest::upstream_proxy::user_context_from_headers(headers))
}

/// The audit context of a request: who, from where, with what client.
pub fn audit_ctx(uc: &crate::upstream::UserContext) -> crate::db::repositories::audit::UserContext {
    crate::db::repositories::audit::UserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

/// `GET /api/health` — database liveness probe.
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
    ok(serde_json::json!({
        "status": status,
        "database": database
    }))
}

#[cfg(test)]
mod serialization_tests {
    use super::*;

    struct Unserializable;

    impl serde::Serialize for Unserializable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("secret detail"))
        }
    }

    /// QUAL-27: a value that cannot be serialized is a 500 with generic text, never a `200` carrying `null`.
    #[test]
    fn an_unserializable_value_is_a_500_not_a_null_200() {
        let (status, Json(body)) = ok_json(Unserializable);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let text = serde_json::to_string(&body).unwrap();
        assert!(!text.contains("secret detail"), "{text}");
        assert!(text.contains("INTERNAL"), "{text}");
    }

    #[test]
    fn a_serializable_value_is_a_200() {
        let (status, Json(body)) = ok_json(serde_json::json!({ "a": 1 }));
        assert_eq!(status, StatusCode::OK);
        assert!(serde_json::to_string(&body).unwrap().contains("\"a\":1"));
    }
}

//! Shared REST handler pieces: the `ApiResponse` helpers (`ok`, `ok_json`), the error mapping
//! (`error_to_response`), the audit-context helpers, and `GET /api/health`. The domain handlers live in the
//! `handlers_*` modules and `upstream_proxy`, wired up in `routes.rs`.

use axa::extract::Json;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::{ApiResponse, problem};

/// Standard handler return type: an `ApiResponse` on success, an RFC 9457 problem otherwise.
pub type HandlerResult = Response;

/// Map a service-layer [`Error`] onto an HTTP status code + machine-readable
/// error code.
pub fn error_to_response(err: Error) -> HandlerResult {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        Error::Validation(_) => (StatusCode::BAD_REQUEST, "validation"),
        Error::Unprocessable(_) => (StatusCode::UNPROCESSABLE_ENTITY, "unprocessable_entity"),
        Error::InvalidStateTransition(_) => (StatusCode::CONFLICT, "invalid_state_transition"),
        Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
        Error::PreconditionFailed(_) => (StatusCode::PRECONDITION_FAILED, "precondition_failed"),
        Error::PreconditionRequired(_) => {
            (StatusCode::PRECONDITION_REQUIRED, "precondition_required")
        }
        Error::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
        Error::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "database"),
        Error::Search(_) => (StatusCode::INTERNAL_SERVER_ERROR, "search"),
        Error::Streaming(_) => (StatusCode::INTERNAL_SERVER_ERROR, "streaming"),
        Error::Fhir(_) => (StatusCode::BAD_REQUEST, "FHIR"),
        Error::Render(_) => (StatusCode::INTERNAL_SERVER_ERROR, "render"),
        Error::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "config"),
        Error::Upstream(_) => (StatusCode::SERVICE_UNAVAILABLE, "upstream_unavailable"),
        Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    problem(status, code, err.client_message()).into_response()
}

/// The `?limit=` of a list request (see [`crate::paging`]).
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

/// `200` with `value` in the `ApiResponse` envelope.
pub fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value))).into_response()
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
        Ok(v) => (StatusCode::OK, Json(ApiResponse::ok_page(v, meta))).into_response(),
        Err(e) => error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        ))),
    }
}

/// `200` with one page of a list (see [`crate::paging`]): at most the page's limit, `meta` saying whether there
/// were more and where the next page is, and the same as `Link: rel="next"`. Ask the service for
/// [`crate::paging::Page::rows`] so there is one row to tell the difference.
pub fn ok_cursor_page<T: serde::Serialize>(
    page: &crate::paging::Page,
    rows: Vec<T>,
) -> HandlerResult {
    let (rows, meta) = page.cut(rows);
    with_next_link(ok_json_page(rows, meta.clone()), &meta, page)
}

/// Add `Link: rel="next"` to a page's response when `meta` says there is a next page.
pub fn with_next_link(
    mut response: HandlerResult,
    meta: &crate::paging::PageMeta,
    page: &crate::paging::Page,
) -> HandlerResult {
    if meta.next.is_some()
        && response.status().is_success()
        && let Some(link) = page.link()
    {
        response
            .headers_mut()
            .insert(axum::http::header::LINK, link);
    }
    response
}

/// `200` with a value that holds a list, and `meta` saying whether that list was cut short: for a body with more
/// in it than the list, such as a summary of every row (QUAL-47).
pub fn ok_json_page<T: serde::Serialize>(value: T, meta: crate::paging::PageMeta) -> HandlerResult {
    match serde_json::to_value(value) {
        Ok(v) => {
            // A list of people is a read of their data (OFH-2): say so, for the read audit.
            let listed = crate::api::rest::read_audit::people_in(&v);
            let mut response =
                (StatusCode::OK, Json(ApiResponse::ok_page(v, meta))).into_response();
            if let Some(n) = listed {
                response
                    .extensions_mut()
                    .insert(crate::api::rest::read_audit::PeopleListed(Some(n)));
            }
            response
        }
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

/// `GET /api/health` — the database and the upstream services.
///
/// `status` is `degraded` when the database or a configured service is not answering, still with HTTP 200,
/// so a probe does not pull a replica out of rotation for a fault every replica shares.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn health(State(state): State<AppState>) -> HandlerResult {
    let db_ok = axa_sqlx::ping(&state.db)
        .await
        .map(|_| true)
        .unwrap_or(false);
    let upstream = state
        .upstream_probe
        .check(state.person_client.as_ref(), state.worker_client.as_ref())
        .await;
    let status = if db_ok && !upstream.is_degraded() {
        "ok"
    } else {
        "degraded"
    };
    let database = if db_ok { "ok" } else { "unreachable" };
    ok(serde_json::json!({
        "status": status,
        "database": database,
        "person_service": upstream.person_service.as_str(),
        "worker_service": upstream.worker_service.as_str(),
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
    #[tokio::test]
    async fn an_unserializable_value_is_a_500_not_a_null_200() {
        let response = ok_json(Unserializable);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let text = body_text(response).await;
        assert!(!text.contains("secret detail"), "{text}");
        assert!(text.contains("internal"), "{text}");
    }

    #[tokio::test]
    async fn a_serializable_value_is_a_200() {
        let response = ok_json(serde_json::json!({ "a": 1 }));
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_text(response).await.contains("\"a\":1"));
    }

    /// FR-206: every kind of error is a problem, with the standard members and a machine-readable `code`.
    #[tokio::test]
    async fn an_error_is_an_rfc_9457_problem_with_a_code() {
        let response = error_to_response(Error::not_found("appointment 1"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            axa_problem_details_for_http_apis::PROBLEM_JSON
        );
        let v: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(v["type"], "about:blank");
        assert_eq!(v["title"], "Not Found");
        assert_eq!(v["status"], 404);
        assert_eq!(v["code"], "not_found");
        assert!(v["detail"].as_str().unwrap().contains("appointment 1"));
        assert!(
            v.get("success").is_none() && v.get("error").is_none(),
            "{v}"
        );
    }

    /// FR-206: what goes wrong inside stays inside; the client's text is the same as it was in the envelope.
    #[tokio::test]
    async fn a_problem_never_carries_internal_failure_text() {
        let response = error_to_response(Error::internal("password=hunter2 at 10.0.0.5"));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let text = body_text(response).await;
        assert!(
            !text.contains("hunter2") && !text.contains("10.0.0.5"),
            "{text}"
        );
    }

    /// A successful body is still the envelope, and no longer carries an always-null `error`.
    #[tokio::test]
    async fn a_success_stays_in_the_envelope() {
        let v: Value = serde_json::from_str(&body_text(ok(serde_json::json!(1))).await).unwrap();
        assert_eq!(v, serde_json::json!({ "success": true, "data": 1 }));
    }

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}

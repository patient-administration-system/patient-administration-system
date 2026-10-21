//! REST handlers for support flags (CAP-10): they only ever add help.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

fn ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub person_id: Uuid,
    pub kind: String,
    #[serde(default)]
    pub appointment_id: Option<Uuid>,
    #[serde(default)]
    pub reason_code: Option<String>,
    pub created_by: String,
}

/// `POST /api/support-flags`: ask for a call or a navigator referral for a patient.
#[utoipa::path(
    post,
    path = "/api/support-flags",
    tag = "Support flags",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<AddBody>,
) -> HandlerResult {
    match state
        .support
        .add(
            b.person_id,
            &b.kind,
            b.appointment_id,
            b.reason_code.as_deref(),
            &b.created_by,
            &ctx(&headers),
        )
        .await
    {
        Ok(id) => ok(serde_json::json!({ "id": id })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenQuery {
    pub limit: Option<i64>,
}

/// `GET /api/support-flags/open`: flags waiting for a person, oldest first.
#[utoipa::path(
    get,
    path = "/api/support-flags/open",
    tag = "Support flags",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn open(State(state): State<AppState>, Query(q): Query<OpenQuery>) -> HandlerResult {
    match state.support.open(q.limit.unwrap_or(100)).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    pub outcome: String,
    pub resolved_by: String,
}

/// `POST /api/support-flags/{id}/resolve`: a named person records what happened.
#[utoipa::path(
    post,
    path = "/api/support-flags/{id}/resolve",
    tag = "Support flags",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<ResolveBody>,
) -> HandlerResult {
    match state
        .support
        .resolve(id, &b.outcome, &b.resolved_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

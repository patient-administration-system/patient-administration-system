//! REST handlers for human follow-up after a missed appointment (CAP-7).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};

/// `GET /api/follow-up-policies`: the services whose misses always get a person's follow-up.
#[utoipa::path(
    get,
    path = "/api/follow-up-policies",
    tag = "Follow up policies",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn policies(State(state): State<AppState>) -> HandlerResult {
    match state.follow_up.policies().await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_followup::ApproveBody)]
pub struct ApproveBody {
    pub approved_by: String,
}

/// `PUT /api/follow-up-policies/{service_type}`: list a service. `approved_by` must name a clinician.
#[utoipa::path(
    put,
    path = "/api/follow-up-policies/{service_type}",
    tag = "Follow up policies",
    params(("service_type" = String, Path, description = "Path parameter `service_type`")),
    request_body = ApproveBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn approve_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service_type): Path<String>,
    Json(body): Json<ApproveBody>,
) -> HandlerResult {
    match state
        .follow_up
        .approve_policy(
            &service_type,
            &body.approved_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct OpenQuery {
    pub limit: Option<i64>,
}

/// `GET /api/follow-up-tasks/open`: misses waiting for a person, most overdue first.
#[utoipa::path(
    get,
    path = "/api/follow-up-tasks/open",
    tag = "Follow up tasks",
    params(OpenQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn open_tasks(
    State(state): State<AppState>,
    Query(q): Query<OpenQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .follow_up
        .open_tasks(crate::paging::ask_for(limit))
        .await
    {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_followup::CloseBody)]
pub struct CloseBody {
    pub outcome: String,
    pub closed_by: String,
}

/// `POST /api/follow-up-tasks/{id}/outcome`: a person records what happened, which closes the task.
#[utoipa::path(
    post,
    path = "/api/follow-up-tasks/{id}/outcome",
    tag = "Follow up tasks",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = CloseBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn close_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<CloseBody>,
) -> HandlerResult {
    match state
        .follow_up
        .close_task(
            id,
            &body.outcome,
            &body.closed_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

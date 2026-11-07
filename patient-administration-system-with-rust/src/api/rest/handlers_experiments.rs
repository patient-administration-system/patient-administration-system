//! REST handlers for wording experiments (CAS-7).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json, ok_page};
use crate::paging;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_experiments::StartBody)]
pub struct StartBody {
    /// The message being tested, for example `appointment_reminder_second`.
    pub template_name: String,
    /// Variant letter to template group name, for example `{"A": "appointment_reminder_second", "B": "second_b"}`.
    pub variants: std::collections::BTreeMap<String, String>,
    pub started_by: String,
}

/// The experiment that was started.
#[derive(Debug, serde::Serialize, Deserialize, utoipa::ToSchema)]
pub struct Started {
    pub id: Uuid,
}

/// `POST /api/experiments`: compare wordings of one message. Each variant must be a complete template group.
#[utoipa::path(
    post,
    path = "/api/experiments",
    tag = "Experiments",
    request_body = StartBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Started>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<StartBody>,
) -> HandlerResult {
    match state
        .experiments
        .start(
            &b.template_name,
            b.variants.into_iter().collect(),
            &b.started_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(id) => ok_json(Started { id }),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListQuery {
    /// How many experiments to return, newest first: 1 to 500, default 200.
    pub limit: Option<i64>,
}

/// `GET /api/experiments`.
#[utoipa::path(
    get,
    path = "/api/experiments",
    tag = "Experiments",
    params(ListQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::experiments::Experiment>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> HandlerResult {
    let limit = match paging::limit(q.limit, 200) {
        Ok(limit) => limit,
        Err(e) => return error_to_response(e),
    };
    match state.experiments.list(paging::ask_for(limit)).await {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StopBody {
    pub stopped_by: String,
}

/// `POST /api/experiments/{id}/stop`.
#[utoipa::path(
    post,
    path = "/api/experiments/{id}/stop",
    tag = "Experiments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = StopBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<StopBody>,
) -> HandlerResult {
    match state
        .experiments
        .stop(id, &b.stopped_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ReadoutQuery {
    pub k: Option<i64>,
}

/// `GET /api/experiments/{id}/readout`: how each wording did by deprivation quintile and language, small
/// cells withheld, no identifiers.
#[utoipa::path(
    get,
    path = "/api/experiments/{id}/readout",
    tag = "Experiments",
    params(("id" = String, Path, description = "Path parameter `id`"), ReadoutQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::experiments::Readout>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn readout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ReadoutQuery>,
) -> HandlerResult {
    match state.experiments.readout(id, q.k).await {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

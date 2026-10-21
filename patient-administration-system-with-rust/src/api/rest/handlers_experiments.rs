//! REST handlers for wording experiments (CAS-7).

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
pub struct StartBody {
    /// The message being tested, for example `appointment_reminder_second`.
    pub template_name: String,
    /// Variant letter to template group name, for example `{"A": "appointment_reminder_second", "B": "second_b"}`.
    pub variants: std::collections::BTreeMap<String, String>,
    pub started_by: String,
}

/// `POST /api/experiments`: compare wordings of one message. Each variant must be a complete template group.
#[utoipa::path(
    post,
    path = "/api/experiments",
    tag = "Experiments",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
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
            &ctx(&headers),
        )
        .await
    {
        Ok(id) => ok(serde_json::json!({ "id": id })),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/experiments`.
#[utoipa::path(
    get,
    path = "/api/experiments",
    tag = "Experiments",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>) -> HandlerResult {
    match state.experiments.list().await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StopBody {
    pub stopped_by: String,
}

/// `POST /api/experiments/{id}/stop`.
#[utoipa::path(
    post,
    path = "/api/experiments/{id}/stop",
    tag = "Experiments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
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
        .stop(id, &b.stopped_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReadoutQuery {
    pub k: Option<i64>,
}

/// `GET /api/experiments/{id}/readout`: how each wording did by deprivation quintile and language, small
/// cells withheld, no identifiers.
#[utoipa::path(
    get,
    path = "/api/experiments/{id}/readout",
    tag = "Experiments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn readout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ReadoutQuery>,
) -> HandlerResult {
    match state.experiments.readout(id, q.k).await {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

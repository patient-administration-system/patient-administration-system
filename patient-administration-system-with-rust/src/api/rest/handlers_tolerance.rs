//! REST handlers for tolerances: how far a measure may drift before it is amber, then red.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::tolerance::{Limits, Scope};

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

/// `GET /api/tolerances`: every measure with its target, tolerance and limit, and whether a person changed it.
#[utoipa::path(
    get,
    path = "/api/tolerances",
    tag = "Tolerances",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>) -> HandlerResult {
    match state.tolerance.settings().await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetBody {
    pub target: f64,
    /// The drift (in the worse direction) that is still green.
    pub tolerance: f64,
    /// The drift past which it is red. Amber is between the two.
    pub escalate_beyond: f64,
    pub set_by: String,
    #[serde(default)]
    pub note: String,
    /// Set for one service only (the service type). Leave out both `service` and `ward` for the whole service.
    pub service: Option<String>,
    /// Set for one ward only (the ward code).
    pub ward: Option<String>,
}

/// `PUT /api/tolerances/{metric}`: a named person sets a measure's tiers.
#[utoipa::path(
    put,
    path = "/api/tolerances/{metric}",
    tag = "Tolerances",
    params(("metric" = String, Path, description = "Path parameter `metric`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(metric): Path<String>,
    Json(b): Json<SetBody>,
) -> HandlerResult {
    let limits = Limits {
        target: b.target,
        tolerance: b.tolerance,
        escalate_beyond: b.escalate_beyond,
    };
    let scope = match Scope::from_parts(b.service.as_deref(), b.ward.as_deref()) {
        Ok(s) => s,
        Err(e) => return error_to_response(e),
    };
    match state
        .tolerance
        .set(&metric, &scope, limits, &b.set_by, &b.note, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ScopeQuery {
    pub service: Option<String>,
    pub ward: Option<String>,
}

/// `DELETE /api/tolerances/{metric}?service=&ward=`: take away what was set. The whole service goes back to
/// the built-in default; a service or ward goes back to the whole service's limits.
#[utoipa::path(
    delete,
    path = "/api/tolerances/{metric}",
    tag = "Tolerances",
    params(("metric" = String, Path, description = "Path parameter `metric`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(metric): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> HandlerResult {
    let scope = match Scope::from_parts(q.service.as_deref(), q.ward.as_deref()) {
        Ok(s) => s,
        Err(e) => return error_to_response(e),
    };
    match state.tolerance.reset(&metric, &scope, &ctx(&headers)).await {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub days: Option<i64>,
}

/// `GET /api/tolerances/status?days=30`: each measure's value and its tier, and the worst tier overall.
#[utoipa::path(
    get,
    path = "/api/tolerances/status",
    tag = "Tolerances",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn status(State(state): State<AppState>, Query(q): Query<StatusQuery>) -> HandlerResult {
    match state.tolerance.status(q.days.unwrap_or(30)).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/tolerances/evaluate?days=30`: check every measure now. A red one opens an alert (once) and a
/// recovered one closes its alert. The server also does this on a timer (`PAS_TOLERANCE_CHECK_MINUTES`).
#[utoipa::path(
    post,
    path = "/api/tolerances/evaluate",
    tag = "Tolerances",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn evaluate(
    State(state): State<AppState>,
    Query(q): Query<StatusQuery>,
) -> HandlerResult {
    match state.tolerance.evaluate(q.days.unwrap_or(30)).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AlertsQuery {
    /// `true` for the alerts still open; the default is all of them.
    pub open: Option<bool>,
}

/// `GET /api/tolerance-alerts?open=true`: alerts, newest first.
#[utoipa::path(
    get,
    path = "/api/tolerance-alerts",
    tag = "Tolerance alerts",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn alerts(State(state): State<AppState>, Query(q): Query<AlertsQuery>) -> HandlerResult {
    match state.tolerance.alerts(q.open.unwrap_or(false)).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AckBody {
    pub acknowledged_by: String,
    #[serde(default)]
    pub note: String,
}

/// `POST /api/tolerance-alerts/{id}/acknowledge`: a named person says they are on it.
#[utoipa::path(
    post,
    path = "/api/tolerance-alerts/{id}/acknowledge",
    tag = "Tolerance alerts",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn acknowledge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(b): Json<AckBody>,
) -> HandlerResult {
    match state
        .tolerance
        .acknowledge(id, &b.acknowledged_by, &b.note, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

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
use crate::tolerance::Limits;

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
}

/// `PUT /api/tolerances/{metric}`: a named person sets a measure's tiers.
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
    match state
        .tolerance
        .set(&metric, limits, &b.set_by, &b.note, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/tolerances/{metric}`: back to the built-in default.
pub async fn reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(metric): Path<String>,
) -> HandlerResult {
    match state.tolerance.reset(&metric, &ctx(&headers)).await {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub days: Option<i64>,
}

/// `GET /api/tolerances/status?days=30`: each measure's value and its tier, and the worst tier overall.
pub async fn status(State(state): State<AppState>, Query(q): Query<StatusQuery>) -> HandlerResult {
    match state.tolerance.status(q.days.unwrap_or(30)).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

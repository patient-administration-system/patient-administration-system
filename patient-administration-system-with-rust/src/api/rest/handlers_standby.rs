//! REST handlers for released slots and the standby list (CAP-3).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::standby::{CandidateFilter, StandbyInput};

fn audit_ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `POST /api/standby`: put a patient on standby for a service (or update their tags).
pub async fn add_standby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<StandbyInput>,
) -> HandlerResult {
    match state.standby.add(input, &audit_ctx(&headers)).await {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListStandbyQuery {
    pub service_type: Option<String>,
}

/// `GET /api/standby?service_type=`: active standby entries, longest-waiting first.
pub async fn list_standby(
    State(state): State<AppState>,
    Query(q): Query<ListStandbyQuery>,
) -> HandlerResult {
    match state.standby.list(q.service_type.as_deref()).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/standby/{id}`: take a patient off standby.
pub async fn remove_standby(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    match state.standby.deactivate(id, &audit_ctx(&headers)).await {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct BookBody {
    pub slot_id: Uuid,
}

/// `POST /api/standby/{id}/book`: book a slot for a patient on standby and take them off the list.
pub async fn book_from_standby(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BookBody>,
) -> HandlerResult {
    match state
        .standby
        .book_from_standby(id, body.slot_id, &audit_ctx(&headers))
        .await
    {
        Ok(a) => ok(serde_json::to_value(a).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReleasesQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// `open`, `refilled` or `expired`.
    pub status: Option<String>,
}

/// `GET /api/slot-releases?from&to&status`: what was released, whether it was refilled and how fast.
/// Defaults to the last 30 days.
pub async fn list_releases(
    State(state): State<AppState>,
    Query(q): Query<ReleasesQuery>,
) -> HandlerResult {
    let to = q.to.unwrap_or_else(|| Utc::now() + Duration::minutes(1));
    let from = q.from.unwrap_or(to - Duration::days(30));
    match state.standby.releases(from, to, q.status.as_deref()).await {
        Ok((summary, releases)) => {
            ok(serde_json::json!({ "summary": summary, "releases": releases }))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/slot-releases/{id}/candidates?language=&needs_transport=&phone_only=`: standby
/// patients who could take this slot, longest-waiting first. Ordered by waiting time and nothing else.
pub async fn release_candidates(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(filter): Query<CandidateFilter>,
) -> HandlerResult {
    match state.standby.candidates(id, &filter).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

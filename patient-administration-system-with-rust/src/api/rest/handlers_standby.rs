//! REST handlers for released slots and the standby list (CAP-3).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};
use crate::standby::{CandidateFilter, Releases, StandbyInput};

/// `POST /api/standby`: put a patient on standby for a service (or update their tags).
#[utoipa::path(
    post,
    path = "/api/standby",
    tag = "Standby",
    request_body = StandbyInput,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::standby::StandbyEntry>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn add_standby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<StandbyInput>,
) -> HandlerResult {
    match state.standby.add(input, &request_audit_ctx(&headers)).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListStandbyQuery {
    pub service_type: Option<String>,
}

/// `GET /api/standby?service_type=`: active standby entries, longest-waiting first.
#[utoipa::path(
    get,
    path = "/api/standby",
    tag = "Standby",
    params(ListStandbyQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::standby::StandbyEntry>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_standby(
    State(state): State<AppState>,
    Query(q): Query<ListStandbyQuery>,
) -> HandlerResult {
    match state.standby.list(q.service_type.as_deref()).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/standby/{id}`: take a patient off standby.
#[utoipa::path(
    delete,
    path = "/api/standby/{id}",
    tag = "Standby",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn remove_standby(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    match state
        .standby
        .deactivate(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_standby::BookBody)]
pub struct BookBody {
    pub slot_id: Uuid,
}

/// `POST /api/standby/{id}/book`: book a slot for a patient on standby and take them off the list.
#[utoipa::path(
    post,
    path = "/api/standby/{id}/book",
    tag = "Standby",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = BookBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::appointment::Appointment>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn book_from_standby(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BookBody>,
) -> HandlerResult {
    match state
        .standby
        .book_from_standby(id, body.slot_id, &request_audit_ctx(&headers))
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ReleasesQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// `open`, `refilled` or `expired`.
    pub status: Option<String>,
}

/// `GET /api/slot-releases?from&to&status`: what was released, whether it was refilled and how fast.
/// Defaults to the last 30 days.
#[utoipa::path(
    get,
    path = "/api/slot-releases",
    tag = "Slot releases",
    params(ReleasesQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::standby::Releases>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_releases(
    State(state): State<AppState>,
    Query(q): Query<ReleasesQuery>,
) -> HandlerResult {
    let to = q.to.unwrap_or_else(|| Utc::now() + Duration::minutes(1));
    let from = q.from.unwrap_or(to - Duration::days(30));
    match state.standby.releases(from, to, q.status.as_deref()).await {
        Ok((summary, releases)) => ok_json(Releases { summary, releases }),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/slot-releases/{id}/candidates?language=&needs_transport=&phone_only=`: standby
/// patients who could take this slot, longest-waiting first. Ordered by waiting time and nothing else.
#[utoipa::path(
    get,
    path = "/api/slot-releases/{id}/candidates",
    tag = "Slot releases",
    params(("id" = String, Path, description = "Path parameter `id`"), CandidateFilter),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::standby::StandbyEntry>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn release_candidates(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(filter): Query<CandidateFilter>,
) -> HandlerResult {
    match state.standby.candidates(id, &filter).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

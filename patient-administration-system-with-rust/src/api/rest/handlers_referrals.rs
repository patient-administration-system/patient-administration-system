//! REST handlers for referrals (CORE-1).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::models::waitlist::Priority;
use crate::referrals::NewReferral;

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
pub struct CreateBody {
    pub person_id: Uuid,
    /// `gp`, `e-rs` or `internal`.
    pub source: String,
    /// The sender's own reference, for example an e-RS UBRN. Required for `e-rs`.
    #[serde(default)]
    pub external_ref: Option<String>,
    pub target_service: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub referring_worker_id: Option<Uuid>,
    #[serde(default)]
    pub received_at: Option<DateTime<Utc>>,
}

/// `POST /api/referrals`: receive a referral. The same `(source, external_ref)` twice is one referral
/// (`duplicate: true`).
#[utoipa::path(
    post,
    path = "/api/referrals",
    tag = "Referrals",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateBody>,
) -> HandlerResult {
    match state
        .referrals
        .create(
            NewReferral {
                person_id: b.person_id,
                source: b.source,
                external_ref: b.external_ref,
                target_service: b.target_service,
                reason: b.reason,
                referring_worker_id: b.referring_worker_id,
                received_at: b.received_at,
            },
            &ctx(&headers),
        )
        .await
    {
        Ok((r, duplicate)) => ok(serde_json::json!({ "referral": r, "duplicate": duplicate })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub service: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/referrals?status=&service=`: oldest first.
#[utoipa::path(
    get,
    path = "/api/referrals",
    tag = "Referrals",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> HandlerResult {
    match state
        .referrals
        .list(
            q.status.as_deref(),
            q.service.as_deref(),
            q.limit.unwrap_or(100),
        )
        .await
    {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/referrals/{id}`.
#[utoipa::path(
    get,
    path = "/api/referrals/{id}",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.referrals.get(id).await {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/referrals/{id}/book-first-available`: book the earliest free slot of the target service and
/// link the appointment. With none free, 409 and the referral stays open.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/book-first-available",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn book_first_available(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .referrals
        .book_first_available(id, &ctx(&headers))
        .await
    {
        Ok(b) => ok(serde_json::to_value(b).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct WaitlistBody {
    #[serde(default)]
    pub priority: Option<Priority>,
}

/// `POST /api/referrals/{id}/waitlist`: put the patient on the waiting list for the service.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/waitlist",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn waitlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<WaitlistBody>>,
) -> HandlerResult {
    let p = body.and_then(|b| b.0.priority).unwrap_or(Priority::Routine);
    match state.referrals.waitlist(id, p, &ctx(&headers)).await {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CloseBody {
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/referrals/{id}/close`.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/close",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<CloseBody>>,
) -> HandlerResult {
    let reason = body.and_then(|b| b.0.reason);
    match state
        .referrals
        .close(id, reason.as_deref(), &ctx(&headers))
        .await
    {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

//! REST handlers for delivery receipts and contact hygiene (NOT-4, CAS-9).

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
pub struct ReceiptBody {
    pub carrier: String,
    /// The carrier's id for this report. The idempotency key with the carrier name.
    pub receipt_id: String,
    /// Our letter id, which we gave the carrier as the message's reference.
    pub letter_id: Uuid,
    /// `delivered`, `bounced` or `replied`.
    pub event: String,
    /// A short reason code for a bounce.
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub occurred_at: Option<DateTime<Utc>>,
}

/// `POST /api/write-back/receipts`: a carrier reports what happened to a message. Carrier-authenticated like
/// the other write-back endpoints; a repeated report changes nothing.
#[utoipa::path(
    post,
    path = "/api/write-back/receipts",
    tag = "Write back",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn post_receipt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<ReceiptBody>,
) -> HandlerResult {
    match state
        .receipts
        .record(
            &b.carrier,
            &b.receipt_id,
            b.letter_id,
            &b.event,
            b.detail.as_deref(),
            b.occurred_at,
            &ctx(&headers),
        )
        .await
    {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letters/{id}/receipts`: what the carrier has said about one message.
#[utoipa::path(
    get,
    path = "/api/letters/{id}/receipts",
    tag = "Letters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn letter_receipts(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.receipts.for_letter(id).await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ProblemsQuery {
    pub since_days: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/contact-problems`: patients whose messages have bounced since a person last dealt with them.
#[utoipa::path(
    get,
    path = "/api/contact-problems",
    tag = "Contact problems",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn contact_problems(
    State(state): State<AppState>,
    Query(q): Query<ProblemsQuery>,
) -> HandlerResult {
    match state
        .receipts
        .contact_problems(q.since_days.unwrap_or(90), q.limit.unwrap_or(100))
        .await
    {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    pub resolved_by: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/contact-problems/{person_id}/resolve`: a person has dealt with the patient's contact details.
#[utoipa::path(
    post,
    path = "/api/contact-problems/{person_id}/resolve",
    tag = "Contact problems",
    params(("person_id" = String, Path, description = "Path parameter `person_id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn resolve_problem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(person_id): Path<Uuid>,
    Json(b): Json<ResolveBody>,
) -> HandlerResult {
    match state
        .receipts
        .resolve_problem(person_id, &b.resolved_by, b.note.as_deref(), &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

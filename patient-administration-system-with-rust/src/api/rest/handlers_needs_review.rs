//! REST handlers for reviewing a patient's communication needs (NEW-20).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_page, request_audit_ctx,
};
use crate::needs_review::{DEFAULT_MONTHS, NeedsReviewService};

#[derive(Debug, Deserialize)]
pub struct ReviewBody {
    /// The person who reviewed the needs with the patient.
    pub reviewed_by: String,
}

/// `POST /api/patients/{id}/communication-needs/review`: record that a named person reviewed the patient's
/// communication needs with them now. Audited.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/communication-needs/review",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record_review(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ReviewBody>,
) -> HandlerResult {
    let at = state.clock.now();
    match NeedsReviewService::new(state.db.clone())
        .record_review(id, &b.reviewed_by, at, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(serde_json::json!({ "person_id": id, "needs_reviewed_at": at })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct DueQuery {
    /// The review period in months (default 12).
    pub months: Option<u32>,
    /// How many to return (1 to 500, default 100); `meta.truncated` says when there were more.
    pub limit: Option<i64>,
}

/// `GET /api/communication-needs/due-review`: patients with recorded communication needs that were never
/// reviewed or not within the period, never-reviewed first.
#[utoipa::path(
    get,
    path = "/api/communication-needs/due-review",
    tag = "Patients",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn due_review(State(state): State<AppState>, Query(q): Query<DueQuery>) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match NeedsReviewService::new(state.db.clone())
        .due(
            state.clock.now(),
            q.months.unwrap_or(DEFAULT_MONTHS),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

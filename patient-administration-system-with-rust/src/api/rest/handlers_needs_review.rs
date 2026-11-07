//! REST handlers for reviewing a patient's communication needs (NEW-20).

use axa::extract::{Json, Path, Query};
use axum::{extract::State, http::HeaderMap};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok, request_audit_ctx};
use crate::needs_review::{DEFAULT_MONTHS, NeedsReviewService};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_needs_review::ReviewBody)]
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
    request_body = ReviewBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DueQuery {
    /// The review period in months (default 12).
    pub months: Option<u32>,
}

/// `GET /api/communication-needs/due-review`: patients with recorded communication needs that were never
/// reviewed or not within the period, never-reviewed first.
#[utoipa::path(
    get,
    path = "/api/communication-needs/due-review",
    tag = "Patients",
    params(crate::paging::PageQuery, DueQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn due_review(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Query(q): Query<DueQuery>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 100) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    match NeedsReviewService::new(state.db.clone())
        .due(
            state.clock.now(),
            q.months.unwrap_or(DEFAULT_MONTHS),
            page.rows(),
        )
        .await
    {
        Ok(rows) => crate::api::rest::handlers::ok_cursor_page(&page, rows),
        Err(e) => error_to_response(e),
    }
}

//! REST handlers for a clinician's priority on a waiting-list entry (NEW-17).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, LimitQuery, error_to_response, ok, ok_page, request_audit_ctx,
};
use crate::clinical_priority::ClinicalPriorityService;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PriorityBody {
    /// P1a, P1b, P2, P3, P4, P5 or P6.
    pub clinical_priority: String,
    /// When a clinician looks at the priority again.
    pub review_by: NaiveDate,
    /// The clinician setting it.
    pub set_by: String,
}

/// `PUT /api/waitlist/{id}/clinical-priority`: a named clinician sets the entry's clinical priority and its
/// review date. Audited. Shown to staff only: nothing orders, offers or books by it (NFR-11).
#[utoipa::path(
    put,
    path = "/api/waitlist/{id}/clinical-priority",
    tag = "Waitlist",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = PriorityBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set_priority(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<PriorityBody>,
) -> HandlerResult {
    match ClinicalPriorityService::new(state.db.clone())
        .set(
            id,
            &b.clinical_priority,
            b.review_by,
            &b.set_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({
            "id": id,
            "clinical_priority": b.clinical_priority,
            "review_by": b.review_by,
        })),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/clinical-priority-reviews/overdue`: entries whose priority review date has passed (clinic
/// date), the longest overdue first; paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/clinical-priority-reviews/overdue",
    tag = "Waitlist",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn overdue(State(state): State<AppState>, Query(q): Query<LimitQuery>) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let today = state
        .clock
        .now()
        .with_timezone(&crate::inbound::messages::timezone())
        .date_naive();
    match ClinicalPriorityService::new(state.db.clone())
        .overdue(today, crate::paging::ask_for(limit))
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

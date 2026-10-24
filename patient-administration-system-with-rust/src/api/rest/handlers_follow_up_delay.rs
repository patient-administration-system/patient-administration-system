//! REST handlers for follow-up target dates and the delayed-follow-up report (NEW-15).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::analytics::AnalyticsService;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, request_audit_ctx,
};
use crate::follow_up_delay::FollowUpDelayService;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TargetBody {
    pub target_date: NaiveDate,
    /// The person setting the target.
    pub set_by: String,
}

/// `PUT /api/waitlist/{id}/follow-up-target`: the date by which a waiting or booked entry's follow-up is
/// due. A named person is required; the change is audited.
#[utoipa::path(
    put,
    path = "/api/waitlist/{id}/follow-up-target",
    tag = "Waitlist",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = TargetBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set_target(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<TargetBody>,
) -> HandlerResult {
    let svc = FollowUpDelayService::new(state.db.clone());
    match svc
        .set_target(id, b.target_date, &b.set_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "follow_up_target": b.target_date })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DelayQuery {
    /// The smallest cell shown; a caller may raise it above the configured minimum, never lower it.
    pub k: Option<i64>,
}

/// `GET /api/reports/follow-up-delay`: follow-up entries past their target, booked and not booked, banded
/// by how far past (DSCN 2020/18), as of the clinic's date. No identifiers; small cells withheld.
#[utoipa::path(
    get,
    path = "/api/reports/follow-up-delay",
    tag = "Reports",
    params(DelayQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn delay_report(
    State(state): State<AppState>,
    Query(q): Query<DelayQuery>,
) -> HandlerResult {
    let k = q.k.unwrap_or(0).max(AnalyticsService::minimum_k());
    let today = state
        .clock
        .now()
        .with_timezone(&crate::inbound::messages::timezone())
        .date_naive();
    match FollowUpDelayService::new(state.db.clone())
        .report(today, k)
        .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

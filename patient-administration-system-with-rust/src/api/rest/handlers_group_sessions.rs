//! REST handlers for group consultations (NEW-18).

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, request_audit_ctx,
};
use crate::group_sessions::GroupSessionService;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CapacityBody {
    /// How many patients the slot takes: 1 for one-to-one, up to 50 for a group session.
    pub capacity: i32,
    pub set_by: String,
}

/// `PUT /api/slots/{id}/capacity`: declare how many patients a slot still to come takes, never below the
/// number already booked. A named person; audited.
/// Needs `If-Match` with the slot's version, the `ETag` of `GET /api/slots/{id}`: none is `428`, a stale one `412` (OFH-10).
#[utoipa::path(
    put,
    path = "/api/slots/{id}/capacity",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = CapacityBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn set_capacity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<CapacityBody>,
) -> HandlerResult {
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "slot") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match GroupSessionService::new(state.db.clone())
        .set_capacity_if_unchanged(
            id,
            b.capacity,
            &b.set_by,
            expected,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "slot_id": id, "capacity": b.capacity })),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/slots/{id}/attendees`: a slot's capacity, how many are booked, and each attendee's appointment
/// and status.
#[utoipa::path(
    get,
    path = "/api/slots/{id}/attendees",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn attendees(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match GroupSessionService::new(state.db.clone()).session(id).await {
        Ok(s) => ok_json(s),
        Err(e) => error_to_response(e),
    }
}

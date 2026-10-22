//! REST handlers for video and phone consultations (CAP-14): the approved-services list and the
//! clinician's review of a conversion.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};
use crate::virtual_visits::Review;

/// `GET /api/virtual-ok-services`: the services a clinician has approved for conversion.
#[utoipa::path(
    get,
    path = "/api/virtual-ok-services",
    tag = "Virtual ok services",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_services(State(state): State<AppState>) -> HandlerResult {
    match state.virtual_visits.list_services().await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ApproveBody {
    pub approved_by: String,
}

/// `PUT /api/virtual-ok-services/{service_type}`: approve a service. `approved_by` must name a person.
#[utoipa::path(
    put,
    path = "/api/virtual-ok-services/{service_type}",
    tag = "Virtual ok services",
    params(("service_type" = String, Path, description = "Path parameter `service_type`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn approve_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service_type): Path<String>,
    Json(body): Json<ApproveBody>,
) -> HandlerResult {
    match state
        .virtual_visits
        .approve_service(
            &service_type,
            &body.approved_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/virtual-ok-services/{service_type}`: stop accepting conversions for a service. Existing
/// conversions stand until reviewed.
#[utoipa::path(
    delete,
    path = "/api/virtual-ok-services/{service_type}",
    tag = "Virtual ok services",
    params(("service_type" = String, Path, description = "Path parameter `service_type`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn remove_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service_type): Path<String>,
) -> HandlerResult {
    match state
        .virtual_visits
        .remove_service(&service_type, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    pub limit: Option<i64>,
}

/// `GET /api/conversions/pending`: conversions waiting for a clinician, soonest appointment first.
#[utoipa::path(
    get,
    path = "/api/conversions/pending",
    tag = "Conversions",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn pending(
    State(state): State<AppState>,
    Query(q): Query<PendingQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .virtual_visits
        .pending(crate::paging::ask_for(limit))
        .await
    {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/{id}/conversion/{decision}`: `confirm` or `veto` (a veto restores in-person).
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/conversion/{decision}",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`"), ("decision" = String, Path, description = "Path parameter `decision`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, decision)): Path<(Uuid, String)>,
) -> HandlerResult {
    let Some(decision) = Review::parse(&decision) else {
        return error_to_response(Error::validation("decision must be confirm or veto"));
    };
    match state
        .virtual_visits
        .review(id, decision, &request_audit_ctx(&headers))
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/{id}/video-failed`: the video visit did not connect. It becomes a phone
/// consultation and a person is asked to ring the patient. Idempotent.
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/video-failed",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn video_failed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .virtual_visits
        .fall_back_to_phone(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

//! REST handlers for video and phone consultations (CAP-14): the approved-services list and the
//! clinician's review of a conversion.

use axa::extract::{Json, Path};
use axum::{extract::State, http::HeaderMap};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};
use crate::virtual_visits::Review;

/// `GET /api/virtual-ok-services`: the services a clinician has approved for conversion.
#[utoipa::path(
    get,
    path = "/api/virtual-ok-services",
    tag = "Virtual ok services",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::virtual_visits::ApprovedService>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list_services(State(state): State<AppState>) -> HandlerResult {
    match state.virtual_visits.list_services().await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_virtual::ApproveBody)]
pub struct ApproveBody {
    pub approved_by: String,
}

/// `PUT /api/virtual-ok-services/{service_type}`: approve a service. `approved_by` must name a person.
#[utoipa::path(
    put,
    path = "/api/virtual-ok-services/{service_type}",
    tag = "Virtual ok services",
    params(("service_type" = String, Path, description = "Path parameter `service_type`")),
    request_body = ApproveBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::virtual_visits::ApprovedService>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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

/// `GET /api/conversions/pending`: conversions waiting for a clinician, soonest appointment first.
#[utoipa::path(
    get,
    path = "/api/conversions/pending",
    tag = "Conversions",
    params(crate::paging::PageQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::appointment::Appointment>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn pending(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 100) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    match state.virtual_visits.pending(page.rows()).await {
        Ok(v) => crate::api::rest::handlers::ok_cursor_page(&page, v),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::appointment::Appointment>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::appointment::Appointment>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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

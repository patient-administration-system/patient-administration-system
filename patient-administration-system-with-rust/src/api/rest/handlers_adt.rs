//! REST handlers for the ADT (admission / discharge / transfer) surface: `POST /api/admissions` and its
//! transfer, discharge and cancel variants on top of [`crate::adt::AdtService`].

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AdmitBody {
    pub person_id: Uuid,
    pub bed_id: Uuid,
}

/// The body of `POST /api/admissions/pre-admit` (OFH-4).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PreAdmitBody {
    pub person_id: Uuid,
    pub bed_id: Uuid,
    /// How many minutes to hold the bed, 1 to 10080. Absent uses `PAS_BED_HOLD_MINUTES`; if that is 0 or unset
    /// the hold does not expire.
    #[serde(default)]
    pub hold_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TransferBody {
    pub new_bed_id: Uuid,
}

/// The answer to cancelling a discharge.
#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Reinstated {
    pub admission_id: Uuid,
    pub reinstated: bool,
}

/// The answer to cancelling a transfer.
#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Reverted {
    pub admission_id: Uuid,
    pub reverted: bool,
}

/// `POST /api/admissions` — admit a person to a bed. Body:
/// `{ "person_id": "<uuid>", "bed_id": "<uuid>" }`.
#[utoipa::path(
    post,
    path = "/api/admissions",
    tag = "Admissions",
    request_body = AdmitBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::adt::AdmissionResult>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdmitBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.admit(body.person_id, body.bed_id, &ctx).await {
        Ok(result) => ok_json(result),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/transfer` — move the patient to a new bed.
/// Body: `{ "new_bed_id": "<uuid>" }`.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/transfer",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = TransferBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::admission::Transfer>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<TransferBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .transfer_if_unchanged(admission_id, body.new_bed_id, expected, &ctx)
        .await
    {
        Ok(transfer) => ok_json(transfer),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/admissions/:id` — one admission. The response carries `ETag`, the version to send back in
/// `If-Match` on a transfer, discharge, leave or cancel of it. The version moves on every one of those (OFH-1).
#[utoipa::path(
    get,
    path = "/api/admissions/{id}",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope. The `ETag` header is the admission's version", body = crate::api::ApiResponse<crate::models::admission::Admission>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn get_admission(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
) -> HandlerResult {
    match crate::db::repositories::admission::AdmissionRepository::find_by_id(
        &state.db,
        admission_id,
    )
    .await
    {
        Ok(Some(a)) => ok_json(a),
        Ok(None) => error_to_response(crate::error::Error::not_found(format!(
            "admission {admission_id}"
        ))),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/discharge` — discharge the open admission.
/// Empty body.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/discharge",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::admission::Discharge>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn discharge(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .discharge_if_unchanged(admission_id, expected, &ctx)
        .await
    {
        Ok(discharge) => ok_json(discharge),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/cancel-admit` — cancel an admission posted
/// in error. Empty body.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/cancel-admit",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::admission::Admission>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn cancel_admit(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .cancel_admission_if_unchanged(admission_id, None, expected, &ctx)
        .await
    {
        Ok(adm) => ok_json(adm),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/cancel-discharge` — reinstate the most-recent
/// discharge on the original bed. Empty body.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/cancel-discharge",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Reinstated>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn cancel_discharge(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .cancel_discharge_if_unchanged(admission_id, expected, &ctx)
        .await
    {
        Ok(_) => ok_json(Reinstated {
            admission_id,
            reinstated: true,
        }),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/cancel-transfer` — reverse the most-recent
/// transfer for the admission.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/cancel-transfer",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Reverted>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn cancel_transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .cancel_transfer_if_unchanged(admission_id, None, expected, &ctx)
        .await
    {
        Ok(_) => ok_json(Reverted {
            admission_id,
            reverted: true,
        }),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/pre-admit` — reserve a bed + open a Planned
/// inpatient encounter (no admission row yet). Body: `{ person_id, bed_id, hold_minutes? }`; with a hold time the bed is given back when it lapses (OFH-4).
#[utoipa::path(
    post,
    path = "/api/admissions/pre-admit",
    tag = "Admissions",
    request_body = PreAdmitBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PreAdmitBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .pre_admit_held(body.person_id, body.bed_id, None, body.hold_minutes, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/cancel-pre-admit` — release the reservation and
/// cancel the Planned inpatient encounter. Body: `{ person_id, bed_id }`.
#[utoipa::path(
    post,
    path = "/api/admissions/cancel-pre-admit",
    tag = "Admissions",
    request_body = AdmitBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn cancel_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdmitBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .cancel_pre_admit(body.person_id, body.bed_id, None, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/leave-start` — patient goes on leave of
/// absence; encounter `InProgress → OnLeave`, bed stays Occupied.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/leave-start",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn leave_start(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .start_leave_if_unchanged(admission_id, None, expected, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/leave-end` — patient returns from LOA;
/// encounter `OnLeave → InProgress`.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/leave-end",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn leave_end(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .end_leave_if_unchanged(admission_id, None, expected, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/change-to-inpatient` — promote the person's
/// most-recent active ambulatory encounter to Inpatient + bed allocation.
/// Body: `{ person_id, bed_id }`.
#[utoipa::path(
    post,
    path = "/api/admissions/change-to-inpatient",
    tag = "Admissions",
    request_body = AdmitBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::adt::AdmissionResult>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn change_to_inpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdmitBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .change_to_inpatient(body.person_id, body.bed_id, None, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/:id/change-to-outpatient` — demote an inpatient
/// admission to Outpatient + release the bed; encounter stays InProgress.
#[utoipa::path(
    post,
    path = "/api/admissions/{id}/change-to-outpatient",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn change_to_outpatient(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let expected = match crate::api::rest::concurrency::require_if_match(&headers, "admission") {
        Ok(t) => t,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .change_to_outpatient_if_unchanged(admission_id, None, expected, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/beds/:id/occupancy-events` — what happened to a bed's hold (reserved, cancelled, expired), newest
/// first; paged (NFR-23) (OFH-4).
#[utoipa::path(
    get,
    path = "/api/beds/{id}/occupancy-events",
    tag = "Admissions",
    params(("id" = String, Path, description = "Path parameter `id`"), crate::api::rest::handlers::LimitQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::adt::OccupancyEvent>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn bed_occupancy_events(
    State(state): State<AppState>,
    Path(bed_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<crate::api::rest::handlers::LimitQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .adt
        .occupancy_events(bed_id, crate::paging::ask_for(limit))
        .await
    {
        Ok(rows) => crate::api::rest::handlers::ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

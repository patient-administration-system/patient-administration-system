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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<TransferBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .transfer(admission_id, body.new_bed_id, &ctx)
        .await
    {
        Ok(transfer) => ok_json(transfer),
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn discharge(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.discharge(admission_id, &ctx).await {
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_admit(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.cancel_admission(admission_id, None, &ctx).await {
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_discharge(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.cancel_discharge(admission_id, &ctx).await {
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.cancel_transfer(admission_id, None, &ctx).await {
        Ok(_) => ok_json(Reverted {
            admission_id,
            reverted: true,
        }),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admissions/pre-admit` — reserve a bed + open a Planned
/// inpatient encounter (no admission row yet). Body: `{ person_id, bed_id }`.
#[utoipa::path(
    post,
    path = "/api/admissions/pre-admit",
    tag = "Admissions",
    request_body = AdmitBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::encounter::Encounter>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdmitBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .pre_admit(body.person_id, body.bed_id, None, &ctx)
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_start(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.start_leave(admission_id, None, &ctx).await {
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_end(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.adt.end_leave(admission_id, None, &ctx).await {
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
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
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn change_to_outpatient(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .adt
        .change_to_outpatient(admission_id, None, &ctx)
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

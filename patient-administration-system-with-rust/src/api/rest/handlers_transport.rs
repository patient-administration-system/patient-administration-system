//! REST handlers for non-emergency transport (CAP-15): staff record eligibility and the patient's consent;
//! the booking itself follows confirmation and needs no call.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct EligibleBody {
    pub eligible_by: String,
    pub basis: String,
}

/// `PUT /api/patients/{id}/transport-eligibility`: staff record that the patient is eligible, and why.
#[utoipa::path(
    put,
    path = "/api/patients/{id}/transport-eligibility",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = EligibleBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set_eligible(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<EligibleBody>,
) -> HandlerResult {
    match state
        .transport
        .set_eligible(id, &b.eligible_by, &b.basis, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/transport-eligibility`.
#[utoipa::path(
    delete,
    path = "/api/patients/{id}/transport-eligibility",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn withdraw_eligible(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .transport
        .withdraw_eligible(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ConsentBody {
    pub recorded_by: String,
    /// The lawful basis, for example "Art. 6(1)(e) and 9(2)(h)".
    pub basis: String,
}

/// `PUT /api/patients/{id}/transport-consent`: the patient's explicit consent to share their pick-up address
/// with the transport provider.
#[utoipa::path(
    put,
    path = "/api/patients/{id}/transport-consent",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ConsentBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record_consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<ConsentBody>,
) -> HandlerResult {
    match state
        .transport
        .record_consent(id, &b.recorded_by, &b.basis, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/transport-consent`: withdraw consent. Later confirmations request no ride.
#[utoipa::path(
    delete,
    path = "/api/patients/{id}/transport-consent",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn withdraw_consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .transport
        .withdraw_consent(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/appointments/{id}/transport`: the ride for this appointment, with its pick-up window.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/transport",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::transport::Booking>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn booking(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.transport.booking_for(id).await {
        Ok(b) => ok_json(b),
        Err(e) => error_to_response(e),
    }
}

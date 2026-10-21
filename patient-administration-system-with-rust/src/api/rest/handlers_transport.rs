//! REST handlers for non-emergency transport (CAP-15): staff record eligibility and the patient's consent;
//! the booking itself follows confirmation and needs no call.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

fn ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

#[derive(Debug, Deserialize)]
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
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
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
        .set_eligible(id, &b.eligible_by, &b.basis, &ctx(&headers))
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
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn withdraw_eligible(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.transport.withdraw_eligible(id, &ctx(&headers)).await {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
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
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
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
        .record_consent(id, &b.recorded_by, &b.basis, &ctx(&headers))
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
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn withdraw_consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.transport.withdraw_consent(id, &ctx(&headers)).await {
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
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn booking(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.transport.booking_for(id).await {
        Ok(b) => ok(serde_json::to_value(b).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

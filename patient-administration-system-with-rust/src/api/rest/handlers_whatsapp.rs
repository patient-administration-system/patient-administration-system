//! REST handlers for the WhatsApp rules (CAP-13): opt-ins and approved templates.

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
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};

fn ok() -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(Value::Null)))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OptInBody {
    /// `patient_reply`, `portal`, `in_person` or `paper`.
    pub source: String,
    pub recorded_by: String,
}

/// `PUT /api/patients/{id}/whatsapp-optin`: the patient has agreed to hear from us on WhatsApp.
#[utoipa::path(
    put,
    path = "/api/patients/{id}/whatsapp-optin",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = OptInBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record_optin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<OptInBody>,
) -> HandlerResult {
    match state
        .whatsapp
        .record_optin(id, &b.source, &b.recorded_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/whatsapp-optin`.
#[utoipa::path(
    delete,
    path = "/api/patients/{id}/whatsapp-optin",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn withdraw_optin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .whatsapp
        .withdraw_optin(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApprovalBody {
    pub provider_template_id: String,
    pub approved_by: String,
}

/// `PUT /api/letter-templates/{id}/whatsapp-approval`: the provider has approved this template for WhatsApp.
#[utoipa::path(
    put,
    path = "/api/letter-templates/{id}/whatsapp-approval",
    tag = "Letter templates",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ApprovalBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn approve_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<ApprovalBody>,
) -> HandlerResult {
    match state
        .whatsapp
        .approve_template(
            id,
            &b.provider_template_id,
            &b.approved_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/letter-templates/{id}/whatsapp-approval`.
#[utoipa::path(
    delete,
    path = "/api/letter-templates/{id}/whatsapp-approval",
    tag = "Letter templates",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn revoke_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .whatsapp
        .revoke_template(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

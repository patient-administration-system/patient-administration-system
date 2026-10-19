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
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn ok() -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(Value::Null)))
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
pub struct OptInBody {
    /// `patient_reply`, `portal`, `in_person` or `paper`.
    pub source: String,
    pub recorded_by: String,
}

/// `PUT /api/patients/{id}/whatsapp-optin`: the patient has agreed to hear from us on WhatsApp.
pub async fn record_optin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<OptInBody>,
) -> HandlerResult {
    match state
        .whatsapp
        .record_optin(id, &b.source, &b.recorded_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/whatsapp-optin`.
pub async fn withdraw_optin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.whatsapp.withdraw_optin(id, &ctx(&headers)).await {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ApprovalBody {
    pub provider_template_id: String,
    pub approved_by: String,
}

/// `PUT /api/letter-templates/{id}/whatsapp-approval`: the provider has approved this template for WhatsApp.
pub async fn approve_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<ApprovalBody>,
) -> HandlerResult {
    match state
        .whatsapp
        .approve_template(id, &b.provider_template_id, &b.approved_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/letter-templates/{id}/whatsapp-approval`.
pub async fn revoke_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.whatsapp.revoke_template(id, &ctx(&headers)).await {
        Ok(()) => ok(),
        Err(e) => error_to_response(e),
    }
}

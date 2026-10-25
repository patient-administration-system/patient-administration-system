//! REST handlers for identity proofing (DTAC-9).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ProofBody {
    pub person_id: Uuid,
    /// `in_person`, `video_id_check`, `nhs_login` or `postal_code`.
    pub method: String,
    pub proofed_by: String,
}

/// `PUT /api/identity-proofings/{subject}`: staff record that they checked this sign-in account belongs to this
/// patient. Audited against the patient.
#[utoipa::path(
    put,
    path = "/api/identity-proofings/{subject}",
    tag = "Identity proofings",
    params(("subject" = String, Path, description = "Path parameter `subject`")),
    request_body = ProofBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Json(b): Json<ProofBody>,
) -> HandlerResult {
    match state
        .identity
        .record(
            &subject,
            b.person_id,
            &b.method,
            &b.proofed_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RevokeQuery {
    pub revoked_by: String,
}

/// `DELETE /api/identity-proofings/{subject}?revoked_by=`: the portal refuses the account from now on.
#[utoipa::path(
    delete,
    path = "/api/identity-proofings/{subject}",
    tag = "Identity proofings",
    params(("subject" = String, Path, description = "Path parameter `subject`"), RevokeQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Query(q): Query<RevokeQuery>,
) -> HandlerResult {
    match state
        .identity
        .revoke(&subject, &q.revoked_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/identity-proofings/{subject}`: the live proofing of an account, for staff.
#[utoipa::path(
    get,
    path = "/api/identity-proofings/{subject}",
    tag = "Identity proofings",
    params(("subject" = String, Path, description = "Path parameter `subject`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(subject): Path<String>) -> HandlerResult {
    match state.identity.get(&subject).await {
        Ok(p) => ok_json(p),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CheckQuery {
    pub person_id: Uuid,
}

/// The answer to the identity check.
#[derive(Debug, serde::Serialize, Deserialize, utoipa::ToSchema)]
pub struct Proofed {
    pub proofed: bool,
}

/// `GET /api/identity-proofings/{subject}/check?person_id=`: `{ "proofed": bool }`, true only for a live
/// proofing of exactly that account and patient. The portal asks this before every patient page.
#[utoipa::path(
    get,
    path = "/api/identity-proofings/{subject}/check",
    tag = "Identity proofings",
    params(("subject" = String, Path, description = "Path parameter `subject`"), CheckQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Proofed>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn check(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Query(q): Query<CheckQuery>,
) -> HandlerResult {
    match state.identity.check(&subject, q.person_id).await {
        Ok(p) => ok_json(Proofed { proofed: p }),
        Err(e) => error_to_response(e),
    }
}

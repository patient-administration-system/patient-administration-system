//! REST handlers for identity proofing (DTAC-9).

use axum::{
    Json,
    extract::{Path, Query, State},
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
pub struct ProofBody {
    pub person_id: Uuid,
    /// `in_person`, `video_id_check`, `nhs_login` or `postal_code`.
    pub method: String,
    pub proofed_by: String,
}

/// `PUT /api/identity-proofings/{subject}`: staff record that they checked this sign-in account belongs to this
/// patient. Audited against the patient.
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
            &ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RevokeQuery {
    pub revoked_by: String,
}

/// `DELETE /api/identity-proofings/{subject}?revoked_by=`: the portal refuses the account from now on.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Query(q): Query<RevokeQuery>,
) -> HandlerResult {
    match state
        .identity
        .revoke(&subject, &q.revoked_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/identity-proofings/{subject}`: the live proofing of an account, for staff.
pub async fn get(State(state): State<AppState>, Path(subject): Path<String>) -> HandlerResult {
    match state.identity.get(&subject).await {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CheckQuery {
    pub person_id: Uuid,
}

/// `GET /api/identity-proofings/{subject}/check?person_id=`: `{ "proofed": bool }`, true only for a live
/// proofing of exactly that account and patient. The portal asks this before every patient page.
pub async fn check(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Query(q): Query<CheckQuery>,
) -> HandlerResult {
    match state.identity.check(&subject, q.person_id).await {
        Ok(p) => ok(serde_json::json!({ "proofed": p })),
        Err(e) => error_to_response(e),
    }
}

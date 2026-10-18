//! REST handlers for contact reliability and communication needs (CAP-4).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::contact::{DEFAULT_STALE_DAYS, ProfileInput};
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn audit_ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `GET /api/patients/{id}/communication-profile`: how to reach this patient well. Includes a
/// `fhir` Patient fragment for systems that speak FHIR.
pub async fn get_profile(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.contact.get(id).await {
        Ok(Some(p)) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!(
            "no communication profile for patient {id}"
        ))),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/patients/{id}/communication-profile`: create or replace. Changing the mobile number
/// clears its verification.
pub async fn put_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> HandlerResult {
    match state.contact.put(id, input, &audit_ctx(&headers)).await {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct VerifyBody {
    /// `staff`, `portal` or `sms_code`.
    pub method: String,
}

/// `POST /api/patients/{id}/communication-profile/verify-mobile`: record that the number was checked.
pub async fn verify_mobile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<VerifyBody>,
) -> HandlerResult {
    match state
        .contact
        .verify_mobile(id, &body.method, &audit_ctx(&headers))
        .await
    {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StaleQuery {
    pub older_than_days: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/communication-profiles/stale?older_than_days=365&limit=100`: profiles whose mobile is
/// missing, unverified or old, oldest first.
pub async fn stale_profiles(
    State(state): State<AppState>,
    Query(q): Query<StaleQuery>,
) -> HandlerResult {
    match state
        .contact
        .stale(
            q.older_than_days.unwrap_or(DEFAULT_STALE_DAYS),
            q.limit.unwrap_or(100),
        )
        .await
    {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

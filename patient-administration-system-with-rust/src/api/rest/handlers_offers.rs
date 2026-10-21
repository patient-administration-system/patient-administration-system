//! REST handlers for backfill offers (PILOT-8).
//!
//! The link endpoints are called by the UI's server for a patient who tapped a link in an offer message.
//! The token is the only thing in the URL and carries no personal data. A GET only reads.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `GET /api/offer-links/{token}`: what the page may show. Read-only.
pub async fn get_offer(State(state): State<AppState>, Path(token): Path<String>) -> HandlerResult {
    match state.backfill.info(&token).await {
        Ok(i) => ok(serde_json::to_value(i).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/offer-links/{token}/{action}`: `accept` or `decline`, a deliberate tap.
pub async fn post_offer_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((token, action)): Path<(String, String)>,
) -> HandlerResult {
    let uc = user_context_from_headers(&headers);
    // The audit trail names the link, not a person: the patient is anonymous until they sign in.
    let ctx = RepoUserContext {
        user_id: Some(format!(
            "offer-link:{}",
            crate::link_token::audit_tag(&token)
        )),
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    let r = match action.as_str() {
        "accept" => state.backfill.accept(&token, &ctx).await,
        "decline" => state.backfill.decline(&token).await,
        _ => Err(Error::validation("action must be accept or decline")),
    };
    match r {
        Ok(o) => ok(serde_json::to_value(o).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/slot-releases/{id}/offers`: who has been offered this slot and how each offer ended.
pub async fn release_offers(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.backfill.offers_for_release(id).await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/slot-releases/{id}/offer`: offer the slot to the next patient on standby now (a person
/// nudging the loop). Does nothing if an offer is already live or nobody is left.
pub async fn offer_now(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.backfill.offer_and_notify(id).await {
        Ok(made) => ok(serde_json::json!({ "offered": made })),
        Err(e) => error_to_response(e),
    }
}

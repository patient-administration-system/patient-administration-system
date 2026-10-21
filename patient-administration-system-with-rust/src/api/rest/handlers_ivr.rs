//! REST handlers for voice reminders with keypad options (CAP-13). Called by the telephony carrier, with the
//! same signed credential as the other write-back endpoints.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

#[derive(Debug, Deserialize)]
pub struct MenuBody {
    /// The single-use token the call was placed with.
    pub token: String,
}

/// `POST /api/write-back/ivr/menu`: what to say when the call connects.
#[utoipa::path(
    post,
    path = "/api/write-back/ivr/menu",
    tag = "Write back",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn menu(State(state): State<AppState>, Json(b): Json<MenuBody>) -> HandlerResult {
    match state.ivr.menu(&b.token).await {
        Ok(r) => (
            StatusCode::OK,
            Json(ApiResponse::ok(
                serde_json::to_value(r).unwrap_or(Value::Null),
            )),
        ),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PressBody {
    pub token: String,
    /// The `state` the last reply gave (`menu`, `times`, `cancel_sure`).
    pub state: String,
    /// One key: 0 to 9.
    pub digit: String,
}

/// `POST /api/write-back/ivr/press`: the patient pressed a key.
#[utoipa::path(
    post,
    path = "/api/write-back/ivr/press",
    tag = "Write back",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn press(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<PressBody>,
) -> HandlerResult {
    let mut chars = b.digit.chars();
    let (Some(d), None) = (chars.next(), chars.next()) else {
        return error_to_response(crate::Error::validation("digit is a single key"));
    };
    let uc = user_context_from_headers(&headers);
    // The audit trail names the call's link, not a person: nobody has signed in.
    let ctx = RepoUserContext {
        user_id: Some(format!("ivr:{}", crate::link_token::audit_tag(&b.token))),
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    match state.ivr.press(&b.token, &b.state, d, &ctx).await {
        Ok(r) => (
            StatusCode::OK,
            Json(ApiResponse::ok(
                serde_json::to_value(r).unwrap_or(Value::Null),
            )),
        ),
        Err(e) => error_to_response(e),
    }
}

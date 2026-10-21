//! REST handlers for reply links (CAP-13) and the staff call list.
//!
//! The link endpoints are called by the UI's server for a patient who has tapped a link in a message. The
//! token is the only thing in the URL; it carries no personal data.

use axum::{
    Json,
    extract::{Path, State},
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
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::reply_links::LinkAction;

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `GET /api/reply-links/{token}`: what the page may show. Read-only: opening a link never acts.
#[utoipa::path(
    get,
    path = "/api/reply-links/{token}",
    tag = "Reply links",
    params(("token" = String, Path, description = "Path parameter `token`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_link(State(state): State<AppState>, Path(token): Path<String>) -> HandlerResult {
    match state.reply_links.info(&token).await {
        Ok(i) => ok(serde_json::to_value(i).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ActBody {
    /// The second step of Cancel.
    #[serde(default)]
    pub confirmed: bool,
    /// With a confirmed cancel: why, if the patient said. Optional; never needed to cancel.
    #[serde(default)]
    pub barrier_code: Option<crate::models::appointment::BarrierCode>,
}

/// `POST /api/reply-links/{token}/{action}`: a deliberate tap. `action` is confirm, cancel, reschedule or
/// help. The first `cancel` answers `needs_confirmation`; send it again with `{"confirmed": true}`.
#[utoipa::path(
    post,
    path = "/api/reply-links/{token}/{action}",
    tag = "Reply links",
    params(("token" = String, Path, description = "Path parameter `token`"), ("action" = String, Path, description = "Path parameter `action`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn post_link_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((token, action)): Path<(String, String)>,
    body: Option<Json<ActBody>>,
) -> HandlerResult {
    let Some(action) = LinkAction::parse(&action) else {
        return error_to_response(Error::validation(
            "action must be confirm, cancel, reschedule or help",
        ));
    };
    let uc = user_context_from_headers(&headers);
    // The audit trail names the link, not a person: the patient is anonymous until they sign in.
    let ctx = RepoUserContext {
        user_id: Some(format!(
            "reply-link:{}",
            crate::link_token::audit_tag(&token)
        )),
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    let body = body.map(|b| b.0).unwrap_or_default();
    match state
        .reply_links
        .act_with(&token, action, body.confirmed, body.barrier_code, &ctx)
        .await
    {
        Ok(o) => ok(serde_json::to_value(o).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChooseBody {
    pub slot_id: Uuid,
}

/// `POST /api/reply-links/{token}/rebook`: the patient chose one of the times offered after Reschedule.
#[utoipa::path(
    post,
    path = "/api/reply-links/{token}/rebook",
    tag = "Reply links",
    params(("token" = String, Path, description = "Path parameter `token`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn post_link_rebook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(body): Json<ChooseBody>,
) -> HandlerResult {
    let uc = user_context_from_headers(&headers);
    let ctx = RepoUserContext {
        user_id: Some(format!(
            "reply-link:{}",
            crate::link_token::audit_tag(&token)
        )),
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    match state.reply_links.choose(&token, body.slot_id, &ctx).await {
        Ok(o) => ok(serde_json::to_value(o).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/staff-call-tasks/open`: patients a person should phone (silent at the second touch, or asked
/// to reschedule), soonest appointment first.
#[utoipa::path(
    get,
    path = "/api/staff-call-tasks/open",
    tag = "Staff call tasks",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn open_calls(State(state): State<AppState>) -> HandlerResult {
    match state.ladder.open_calls(200).await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CallOutcomeBody {
    pub outcome: String,
}

/// `POST /api/staff-call-tasks/{id}/outcome`: record what happened on the call; it leaves the list.
#[utoipa::path(
    post,
    path = "/api/staff-call-tasks/{id}/outcome",
    tag = "Staff call tasks",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn call_outcome(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<CallOutcomeBody>,
) -> HandlerResult {
    let uc = user_context_from_headers(&headers);
    match state
        .ladder
        .complete_call(id, &b.outcome, uc.user_id.as_deref())
        .await
    {
        Ok(true) => ok(serde_json::json!({ "handled": true })),
        Ok(false) => error_to_response(Error::not_found("no open call task with that id")),
        Err(e) => error_to_response(e),
    }
}

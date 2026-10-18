//! REST handlers for waitlist + RTT.

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
use crate::models::waitlist::Priority;

#[derive(Debug, Deserialize)]
pub struct WaitlistAddBody {
    pub person_id: Uuid,
    pub target_service: String,
    pub priority: Priority,
    #[serde(default)]
    pub referral_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListByServiceQuery {
    pub service: String,
}

#[derive(Debug, Deserialize)]
pub struct RttStartBody {
    pub person_id: Uuid,
    pub target_service: String,
}

#[derive(Debug, Deserialize)]
pub struct RttReasonBody {
    pub reason: String,
}

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

// ─── Waitlist ──────────────────────────────────────────────────────────

/// `POST /api/waitlist`.
pub async fn add_to_waitlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WaitlistAddBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .waitlist
        .add(
            body.referral_id,
            body.person_id,
            body.target_service,
            body.priority,
            &ctx,
        )
        .await
    {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/waitlist/:id`.
pub async fn remove_from_waitlist(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.waitlist.remove(entry_id, &ctx).await {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/waitlist?service=cardiology`.
pub async fn list_waitlist(
    State(state): State<AppState>,
    Query(q): Query<ListByServiceQuery>,
) -> HandlerResult {
    match state.waitlist.list_by_service(&q.service).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BookFromEntryBody {
    pub slot_id: Uuid,
}

/// `POST /api/waitlist/{id}/book`: book a slot for the patient on this entry, and mark the entry
/// `booked`. A cancel or a miss later returns it to `waiting`; completing the appointment marks it
/// `treated`.
pub async fn book_from_waitlist(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BookFromEntryBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .scheduling
        .book_for_entry(entry_id, body.slot_id, &ctx)
        .await
    {
        Ok(a) => ok(serde_json::to_value(a).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

// ─── RTT ───────────────────────────────────────────────────────────────

/// `POST /api/rtt/start`.
pub async fn rtt_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RttStartBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .rtt
        .start(body.person_id, body.target_service, &ctx)
        .await
    {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/pause`.
pub async fn rtt_pause(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RttReasonBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.pause(pathway_id, body.reason, &ctx).await {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/resume`.
pub async fn rtt_resume(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.resume(pathway_id, &ctx).await {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/stop`.
pub async fn rtt_stop(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RttReasonBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.stop(pathway_id, body.reason, &ctx).await {
        Ok(e) => ok(serde_json::to_value(e).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/rtt/:id/weeks-waiting`.
pub async fn rtt_weeks_waiting(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
) -> HandlerResult {
    match state.rtt.weeks_waiting(pathway_id).await {
        Ok(w) => ok(serde_json::json!({ "pathway_id": pathway_id, "weeks_waiting": w })),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/rtt/:id/breaching`.
pub async fn rtt_is_breaching(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
) -> HandlerResult {
    match state.rtt.is_breaching(pathway_id).await {
        Ok(b) => ok(serde_json::json!({ "pathway_id": pathway_id, "breaching": b })),
        Err(e) => error_to_response(e),
    }
}

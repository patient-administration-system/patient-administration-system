//! REST handlers for waitlist + RTT.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::models::waitlist::Priority;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct WaitlistAddBody {
    pub person_id: Uuid,
    pub target_service: String,
    pub priority: Priority,
    #[serde(default)]
    pub referral_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListByServiceQuery {
    pub service: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RttStartBody {
    pub person_id: Uuid,
    pub target_service: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RttReasonBody {
    pub reason: String,
}

// ─── Waitlist ──────────────────────────────────────────────────────────

/// `POST /api/waitlist`.
#[utoipa::path(
    post,
    path = "/api/waitlist",
    tag = "Waitlist",
    request_body = WaitlistAddBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::waitlist::WaitlistEntry>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/waitlist/:id`.
#[utoipa::path(
    delete,
    path = "/api/waitlist/{id}",
    tag = "Waitlist",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::waitlist::WaitlistEntry>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn remove_from_waitlist(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.waitlist.remove(entry_id, &ctx).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/waitlist?service=cardiology`.
#[utoipa::path(
    get,
    path = "/api/waitlist",
    tag = "Waitlist",
    params(ListByServiceQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::waitlist::WaitlistEntry>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_waitlist(
    State(state): State<AppState>,
    Query(q): Query<ListByServiceQuery>,
) -> HandlerResult {
    match state.waitlist.list_by_service(&q.service).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct BookFromEntryBody {
    pub slot_id: Uuid,
}

/// `POST /api/waitlist/{id}/book`: book a slot for the patient on this entry, and mark the entry
/// `booked`. A cancel or a miss later returns it to `waiting`; completing the appointment marks it
/// `treated`.
#[utoipa::path(
    post,
    path = "/api/waitlist/{id}/book",
    tag = "Waitlist",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = BookFromEntryBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::appointment::Appointment>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

// ─── RTT ───────────────────────────────────────────────────────────────

/// `POST /api/rtt/start`.
#[utoipa::path(
    post,
    path = "/api/rtt/start",
    tag = "Rtt",
    request_body = RttStartBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::rtt::RTTPathway>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
        Ok(p) => ok_json(p),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/pause`.
#[utoipa::path(
    post,
    path = "/api/rtt/{id}/pause",
    tag = "Rtt",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = RttReasonBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::rtt::RTTClockEvent>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rtt_pause(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RttReasonBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.pause(pathway_id, body.reason, &ctx).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/resume`.
#[utoipa::path(
    post,
    path = "/api/rtt/{id}/resume",
    tag = "Rtt",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::rtt::RTTClockEvent>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rtt_resume(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.resume(pathway_id, &ctx).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rtt/:id/stop`.
#[utoipa::path(
    post,
    path = "/api/rtt/{id}/stop",
    tag = "Rtt",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = RttReasonBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::rtt::RTTClockEvent>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rtt_stop(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RttReasonBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.rtt.stop(pathway_id, body.reason, &ctx).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/rtt/:id/weeks-waiting`.
#[utoipa::path(
    get,
    path = "/api/rtt/{id}/weeks-waiting",
    tag = "Rtt",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
#[utoipa::path(
    get,
    path = "/api/rtt/{id}/breaching",
    tag = "Rtt",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rtt_is_breaching(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
) -> HandlerResult {
    match state.rtt.is_breaching(pathway_id).await {
        Ok(b) => ok(serde_json::json!({ "pathway_id": pathway_id, "breaching": b })),
        Err(e) => error_to_response(e),
    }
}

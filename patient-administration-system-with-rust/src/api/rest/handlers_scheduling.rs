//! REST handlers for the scheduling surface.
//!
//! Phase 3b1 restores slot booking + the appointment lifecycle on top of
//! the existing [`crate::scheduling::SchedulingService`]. Bulk slot
//! generation, appointment series, and the `mark_no_show` path land in
//! Phase 3b2.

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
use crate::models::appointment::CancellationReason;

#[derive(Debug, Deserialize)]
pub struct BookBody {
    pub person_id: Uuid,
}

#[derive(Debug, Deserialize, Default)]
pub struct CancelBody {
    #[serde(default)]
    pub reason: Option<String>,
}

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok_value(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `POST /api/slots/:id/book` — book the named slot for the supplied
/// person id. Body: `{ "person_id": "<uuid>" }`.
pub async fn book_slot(
    State(state): State<AppState>,
    Path(slot_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BookBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .scheduling
        .book_slot(slot_id, body.person_id, &ctx)
        .await
    {
        Ok(appt) => ok_value(serde_json::to_value(appt).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/cancel` — cancel a booked appointment.
/// Body: `{ "reason": "<free text>" }` (optional; defaults to
/// `PatientRequest`).
pub async fn cancel_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    body: Option<Json<CancelBody>>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    // `reason` body field is reserved for Phase 3b2 (when we add a
    // free-text reason column or extend the enum). For Phase 3b1 every
    // cancel maps to `PatientRequest`.
    let _ = body.and_then(|Json(b)| b.reason);
    let reason = CancellationReason::PatientRequest;
    match state.scheduling.cancel(appointment_id, reason, &ctx).await {
        Ok(appt) => ok_value(serde_json::to_value(appt).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/check-in` — `Booked → Arrived`.
pub async fn check_in_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.check_in(appointment_id, &ctx).await {
        Ok(appt) => ok_value(serde_json::to_value(appt).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/complete` — `Arrived → Fulfilled`.
pub async fn complete_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.complete(appointment_id, &ctx).await {
        Ok(appt) => ok_value(serde_json::to_value(appt).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/no-show` — `Booked → NoShow`.
pub async fn mark_no_show(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.mark_no_show(appointment_id, &ctx).await {
        Ok(appt) => ok_value(serde_json::to_value(appt).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

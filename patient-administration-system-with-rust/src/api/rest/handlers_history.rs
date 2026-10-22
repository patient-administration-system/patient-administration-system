//! REST handlers for explaining a change or a contact (CAP-6). Read-only.

use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::ok;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page};

/// `GET /api/appointments/{id}/history`: who did what to this appointment, when and why, with the messages
/// sent about it and the patient's replies, oldest first.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/history",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn appointment_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.history.appointment_history(id).await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub limit: Option<i64>,
}

/// `GET /api/patients/{id}/contact-log`: every message sent to the patient and every text received, newest
/// first, with what triggered each. It says that a contact happened and why, never what it said.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/contact-log",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn contact_log(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LogQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 200) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .history
        .contact_log(id, crate::paging::ask_for(limit))
        .await
    {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

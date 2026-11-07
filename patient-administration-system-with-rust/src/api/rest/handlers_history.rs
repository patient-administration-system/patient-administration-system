//! REST handlers for explaining a change or a contact (CAP-6). Read-only.

use axa::extract::Path;
use axum::extract::State;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json};

/// `GET /api/appointments/{id}/history`: who did what to this appointment, when and why, with the messages
/// sent about it and the patient's replies, oldest first.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/history",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::history::HistoryRow>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn appointment_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.history.appointment_history(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/{id}/contact-log`: every message sent to the patient and every text received, newest
/// first, with what triggered each. It says that a contact happened and why, never what it said.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/contact-log",
    tag = "Patients",
    params(crate::paging::PageQuery, ("id" = String, Path, description = "Path parameter `id`"), ),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::history::HistoryRow>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn contact_log(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 200) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    match state.history.contact_log(id, page.rows()).await {
        Ok(v) => crate::api::rest::handlers::ok_cursor_page(&page, v),
        Err(e) => error_to_response(e),
    }
}

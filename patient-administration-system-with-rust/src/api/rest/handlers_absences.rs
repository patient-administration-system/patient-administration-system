//! REST handlers for staff absences (TSC-10). The person acting is the signed-in user; recording and
//! withdrawing are audited.

use axa::extract::{Json, Path};
use axum::{extract::State, http::HeaderMap};
use uuid::Uuid;

use crate::absences::{AbsenceService, NewAbsence};
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json, request_audit_ctx};

fn who(headers: &HeaderMap) -> String {
    // RCA-DECL-1: no caller is blank, which the service refuses (a named person is required).
    request_audit_ctx(headers).user_id.unwrap_or(String::new())
}

/// `POST /api/absences`: a clinician or room is unavailable for a window (no reason is asked or kept). Blocks
/// their free slots in it and makes a work item to rebook each booked appointment it touches; nothing is
/// cancelled and no patient is told.
#[utoipa::path(
    post,
    path = "/api/absences",
    tag = "Scheduling",
    request_body = NewAbsence,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::absences::Absence),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn record(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NewAbsence>,
) -> HandlerResult {
    match AbsenceService::new(state.db.clone())
        .record(body, &who(&headers), &request_audit_ctx(&headers))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/absences/{id}`: one absence, the slots it blocked and the appointments it touched.
#[utoipa::path(
    get,
    path = "/api/absences/{id}",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::absences::Absence),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match AbsenceService::new(state.db.clone()).get(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/absences/{id}/withdraw`: the absence will not happen. The slots it blocked are free again and
/// its work items close.
#[utoipa::path(
    post,
    path = "/api/absences/{id}/withdraw",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::absences::Absence),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn withdraw(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match AbsenceService::new(state.db.clone())
        .withdraw(id, &who(&headers), &request_audit_ctx(&headers))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

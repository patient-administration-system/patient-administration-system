//! REST handlers for encounter reads + lifecycle.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::error::LogErr;
use crate::models::encounter::EncounterStatus;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetStatusBody {
    pub status: EncounterStatus,
}

/// `GET /api/encounters/:id`.
#[utoipa::path(
    get,
    path = "/api/encounters/{id}",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_encounter(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match EncounterRepository::find_by_id(&state.db, id).await {
        Ok(Some(e)) => ok_json(e),
        Ok(None) => error_to_response(Error::not_found(format!("encounter {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/:id/encounters` — list a person's encounters.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/encounters",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_encounters_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match EncounterRepository::list_by_patient(&state.db, person_id).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/encounters/:id/status` — state-machine-checked status flip.
#[utoipa::path(
    put,
    path = "/api/encounters/{id}/status",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = SetStatusBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SetStatusBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match EncounterRepository::set_status(&state.db, id, body.status).await {
        Ok(e) => {
            AuditLogRepository::log(
                &state.db,
                "encounter",
                id,
                "set_status",
                None,
                Some(serde_json::to_value(&e).unwrap_or_default()),
                &ctx,
            )
            .await
            .log_error("audit log write");
            ok_json(e)
        }
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/encounters/:id/cancel` — convenience: set status to
/// Cancelled. Equivalent to `PUT /api/encounters/:id/status` with
/// `{"status":"Cancelled"}`.
#[utoipa::path(
    post,
    path = "/api/encounters/{id}/cancel",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_encounter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match EncounterRepository::set_status(&state.db, id, EncounterStatus::Cancelled).await {
        Ok(e) => {
            AuditLogRepository::log(
                &state.db,
                "encounter",
                id,
                "cancel",
                None,
                Some(serde_json::to_value(&e).unwrap_or_default()),
                &ctx,
            )
            .await
            .log_error("audit log write");
            ok_json(e)
        }
        Err(e) => error_to_response(e),
    }
}

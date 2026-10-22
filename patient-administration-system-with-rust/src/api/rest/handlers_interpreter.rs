//! REST handlers for interpreting needs and interpreter requests (NEW-5).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_page, request_audit_ctx,
};
use crate::interpreter::InterpreterService;

#[derive(Debug, Deserialize)]
pub struct NeedBody {
    /// `spoken`, `bsl`, `deafblind`, or null to clear.
    pub kind: Option<String>,
    /// For a spoken-language interpreter: the language (a BCP 47 tag such as `pl` or `ar`).
    pub language: Option<String>,
    pub recorded_by: String,
}

/// `PUT /api/patients/{id}/interpreter-need`: what interpreting the patient needs, or none. A named person;
/// audited. Every later booking for the patient raises an interpreter request.
#[utoipa::path(
    put,
    path = "/api/patients/{id}/interpreter-need",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn set_need(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<NeedBody>,
) -> HandlerResult {
    match InterpreterService::new(state.db.clone())
        .set_need(
            id,
            b.kind.as_deref(),
            b.language.as_deref(),
            &b.recorded_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => {
            ok(serde_json::json!({ "person_id": id, "kind": b.kind, "language": b.language }))
        }
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmBody {
    pub confirmed_by: String,
    /// The interpreting service's booking reference, when it gave one.
    pub booking_ref: Option<String>,
}

/// `POST /api/interpreter-requests/{id}/confirm`: a named person confirms the interpreter is booked.
#[utoipa::path(
    post,
    path = "/api/interpreter-requests/{id}/confirm",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn confirm(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ConfirmBody>,
) -> HandlerResult {
    match InterpreterService::new(state.db.clone())
        .confirm(
            id,
            &b.confirmed_by,
            b.booking_ref.as_deref(),
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "status": "confirmed" })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct UnconfirmedQuery {
    /// How far ahead to look, in hours (1 to 336, default 36: tomorrow's clinics, checked today).
    pub within_hours: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/interpreter-requests/unconfirmed`: booked appointments starting soon whose interpreter is not
/// confirmed, soonest first; paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/interpreter-requests/unconfirmed",
    tag = "Scheduling",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn unconfirmed(
    State(state): State<AppState>,
    Query(q): Query<UnconfirmedQuery>,
) -> HandlerResult {
    let hours = q.within_hours.unwrap_or(36);
    if !(1..=336).contains(&hours) {
        return error_to_response(crate::Error::validation(
            "within_hours must be between 1 and 336",
        ));
    }
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match InterpreterService::new(state.db.clone())
        .unconfirmed(
            state.clock.now(),
            chrono::Duration::hours(hours),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

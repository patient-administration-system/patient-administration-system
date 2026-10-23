//! REST handlers for reasonable adjustments (NEW-6).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::adjustments::{AdjustmentService, CATALOGUE, NewAdjustment};
use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, ok_page, request_audit_ctx,
};

#[derive(Debug, Deserialize)]
pub struct RecordBody {
    /// A code from `GET /api/reasonable-adjustments/codes`.
    pub code: String,
    /// A short note (at most 200 characters); no more than staff need to make the adjustment.
    pub note: Option<String>,
    /// When a person should look at the adjustment again.
    pub review_by: Option<NaiveDate>,
    pub recorded_by: String,
}

/// `POST /api/patients/{id}/reasonable-adjustments`: a named person records an adjustment. Audited. The
/// answer carries the adjustment as a FHIR `Flag`.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/reasonable-adjustments",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<RecordBody>,
) -> HandlerResult {
    match AdjustmentService::new(state.db.clone())
        .record(
            id,
            &NewAdjustment {
                code: &b.code,
                note: b.note.as_deref(),
                review_by: b.review_by,
                recorded_by: &b.recorded_by,
            },
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Include ended adjustments (default false: what staff need to act on).
    pub include_ended: Option<bool>,
    pub limit: Option<i64>,
}

/// `GET /api/patients/{id}/reasonable-adjustments`: the patient's live adjustments (and ended ones on
/// request), each with its FHIR `Flag`; what the staff booking view shows. Paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/patients/{id}/reasonable-adjustments",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match AdjustmentService::new(state.db.clone())
        .for_patient(
            id,
            q.include_ended.unwrap_or(false),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/reasonable-adjustments/codes`: the catalogue of codes, with category and display text.
#[utoipa::path(
    get,
    path = "/api/reasonable-adjustments/codes",
    tag = "Patients",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn codes() -> HandlerResult {
    ok(serde_json::json!({
        "system": crate::adjustments::CODE_SYSTEM,
        "codes": CATALOGUE
            .iter()
            .map(|(category, code, display)| serde_json::json!({ "category": category, "code": code, "display": display }))
            .collect::<Vec<_>>(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct EndBody {
    pub ended_by: String,
}

/// `POST /api/reasonable-adjustments/{id}/end`: a named person ends a live adjustment. Audited.
#[utoipa::path(
    post,
    path = "/api/reasonable-adjustments/{id}/end",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn end(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<EndBody>,
) -> HandlerResult {
    match AdjustmentService::new(state.db.clone())
        .end(
            id,
            &b.ended_by,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "status": "ended" })),
        Err(e) => error_to_response(e),
    }
}

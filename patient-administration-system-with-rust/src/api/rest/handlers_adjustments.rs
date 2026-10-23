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
pub struct IdsQuery {
    /// Comma-separated appointment ids, at most 500.
    pub ids: String,
}

/// `GET /api/reasonable-adjustments/for-appointments?ids=a,b,c`: the live adjustments of each appointment's
/// patient, for the staff schedule: `[{appointment_id, adjustments: [{code, category, display}]}]`, only for
/// appointments whose patient has any.
#[utoipa::path(
    get,
    path = "/api/reasonable-adjustments/for-appointments",
    tag = "Patients",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn for_appointments(
    State(state): State<AppState>,
    Query(q): Query<IdsQuery>,
) -> HandlerResult {
    let ids: Vec<Uuid> = q
        .ids
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    match AdjustmentService::new(state.db.clone())
        .live_for_appointments(&ids)
        .await
    {
        Ok(pairs) => {
            let mut out: Vec<serde_json::Value> = Vec::new();
            let mut last: Option<Uuid> = None;
            for (appointment_id, code) in pairs {
                let (category, display) = crate::adjustments::lookup(&code).unwrap_or(("", ""));
                let item =
                    serde_json::json!({ "code": code, "category": category, "display": display });
                if last == Some(appointment_id)
                    && let Some(o) = out.last_mut()
                    && let Some(list) = o["adjustments"].as_array_mut()
                {
                    list.push(item);
                } else {
                    out.push(serde_json::json!({ "appointment_id": appointment_id, "adjustments": [item] }));
                    last = Some(appointment_id);
                }
            }
            ok(serde_json::Value::Array(out))
        }
        Err(e) => error_to_response(e),
    }
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

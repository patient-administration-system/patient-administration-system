//! REST handlers for wait-time and lead-time data (CAP-8).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};

/// `GET /api/appointments/{id}/lead-time`: booking to visit, in whole days.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/lead-time",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn lead_time(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.waits.lead_time(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/{id}/waits`: the lead time of each appointment and the weeks waited on each pathway.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/waits",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn patient_waits(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.waits.patient_waits(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct DueQuery {
    pub weeks: Option<i64>,
    pub repeat_days: Option<i64>,
}

/// `GET /api/pathways/due-check?weeks=26&repeat_days=90`: long waiters nobody has asked recently whether
/// they still need this, longest first.
#[utoipa::path(
    get,
    path = "/api/pathways/due-check",
    tag = "Pathways",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn due_for_check(
    State(state): State<AppState>,
    Query(q): Query<DueQuery>,
) -> HandlerResult {
    match state.waits.due_for_check(q.weeks, q.repeat_days).await {
        Ok(v) => {
            let mut rows = Vec::with_capacity(v.len());
            for (person_id, p) in v {
                match serde_json::to_value(p) {
                    Ok(mut j) => {
                        j["person_id"] = serde_json::json!(person_id);
                        rows.push(j);
                    }
                    Err(e) => {
                        return error_to_response(crate::Error::internal(format!(
                            "response could not be serialized: {e}"
                        )));
                    }
                }
            }
            ok(Value::Array(rows))
        }
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AskBody {
    pub asked_by: String,
}

/// `POST /api/pathways/{id}/still-needed-checks`: a person asks the patient whether they still need this.
#[utoipa::path(
    post,
    path = "/api/pathways/{id}/still-needed-checks",
    tag = "Pathways",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<AskBody>,
) -> HandlerResult {
    match state
        .waits
        .ask(id, &b.asked_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(check) => ok(serde_json::json!({ "id": check })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    pub answer: String,
    pub recorded_by: String,
}

/// `POST /api/still-needed-checks/{id}/answer`: record the answer. It changes nothing else.
#[utoipa::path(
    post,
    path = "/api/still-needed-checks/{id}/answer",
    tag = "Still needed checks",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<AnswerBody>,
) -> HandlerResult {
    match state
        .waits
        .answer(id, &b.answer, &b.recorded_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

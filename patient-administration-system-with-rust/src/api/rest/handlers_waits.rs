//! REST handlers for wait-time and lead-time data (CAP-8).

use axum::{
    Json,
    extract::{Path, Query, State},
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

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

fn ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

/// `GET /api/appointments/{id}/lead-time`: booking to visit, in whole days.
pub async fn lead_time(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.waits.lead_time(id).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/{id}/waits`: the lead time of each appointment and the weeks waited on each pathway.
pub async fn patient_waits(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.waits.patient_waits(id).await {
        Ok(v) => ok(serde_json::to_value(v).unwrap_or(Value::Null)),
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
pub async fn due_for_check(
    State(state): State<AppState>,
    Query(q): Query<DueQuery>,
) -> HandlerResult {
    match state.waits.due_for_check(q.weeks, q.repeat_days).await {
        Ok(v) => ok(Value::Array(
            v.into_iter()
                .map(|(person_id, p)| {
                    let mut j = serde_json::to_value(p).unwrap_or(Value::Null);
                    j["person_id"] = serde_json::json!(person_id);
                    j
                })
                .collect(),
        )),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AskBody {
    pub asked_by: String,
}

/// `POST /api/pathways/{id}/still-needed-checks`: a person asks the patient whether they still need this.
pub async fn ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<AskBody>,
) -> HandlerResult {
    match state.waits.ask(id, &b.asked_by, &ctx(&headers)).await {
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
pub async fn answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<AnswerBody>,
) -> HandlerResult {
    match state
        .waits
        .answer(id, &b.answer, &b.recorded_by, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

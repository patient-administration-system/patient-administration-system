//! REST handlers for the no-show probability (CAP-12). Support only, and off until people approve it.

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

/// `GET /api/noshow/status`: whether it is on, and if not, exactly why.
pub async fn status(State(state): State<AppState>) -> HandlerResult {
    match state.noshow.status().await {
        Ok(s) => ok(serde_json::to_value(s).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/noshow/model-card`: what the model is, its weights and what it must never do.
pub async fn model_card() -> HandlerResult {
    ok(crate::noshow::NoShowService::model_card())
}

#[derive(Debug, Deserialize)]
pub struct ReviewBody {
    /// `fairness` or `clinical_safety`.
    pub kind: String,
    pub reviewer: String,
    /// `approved` or `rejected`. A rejection switches the score off at once.
    pub outcome: String,
    pub notes: String,
}

/// `POST /api/noshow/reviews`: a named person approves or rejects this model version.
pub async fn record_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<ReviewBody>,
) -> HandlerResult {
    match state
        .noshow
        .record_review(&b.kind, &b.reviewer, &b.outcome, &b.notes, &ctx(&headers))
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub days: Option<i64>,
    pub k: Option<i64>,
}

/// `GET /api/noshow/fairness-audit`: the model backtested on past outcomes by deprivation quintile and
/// language, small cells withheld. Evidence for the fairness review; it works while the score is off.
pub async fn fairness_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> HandlerResult {
    match state
        .noshow
        .fairness_audit(q.days.unwrap_or(365), q.k)
        .await
    {
        Ok(v) => ok(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/appointments/{id}/noshow`: the probability, band and the factors behind it (409 while off).
pub async fn for_appointment(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.noshow.for_staff(id).await {
        Ok(s) => ok(serde_json::to_value(s).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct IdsQuery {
    /// Comma-separated appointment ids, at most 500.
    pub ids: String,
}

/// `GET /api/noshow/for-appointments?ids=a,b,c`: the stored assessments for many appointments at once, for a
/// schedule view. `{"active": false, "items": []}` while the score is off; never an error for that.
pub async fn for_appointments(
    State(state): State<AppState>,
    Query(q): Query<IdsQuery>,
) -> HandlerResult {
    let ids: Vec<Uuid> = q
        .ids
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    match state.noshow.for_appointments(&ids).await {
        Ok((active, items)) => ok(serde_json::json!({ "active": active, "items": items })),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/noshow/recalculate`: the daily job, on demand. Idempotent within a day.
pub async fn recalculate(State(state): State<AppState>) -> HandlerResult {
    match state.noshow.recalculate_all().await {
        Ok(n) => ok(serde_json::json!({ "assessed": n })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
}

/// `GET /api/noshow/priority-list`: appointments above the high threshold that nobody has dealt with yet,
/// soonest first, with the factors. Clear one by recording an outcome on its task
/// (`POST /api/staff-call-tasks/{task_id}/outcome`).
pub async fn priority_list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> HandlerResult {
    match state.noshow.priority_list(q.limit.unwrap_or(100)).await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/noshow/rollout`: each clinic's stage and how long it has been recording in shadow.
pub async fn rollout(State(state): State<AppState>) -> HandlerResult {
    match state.noshow.rollout().await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StageBody {
    /// `shadow` or `live`.
    pub stage: String,
    pub changed_by: String,
    pub note: String,
}

/// `PUT /api/noshow/rollout/{service_type}`: a named person moves one clinic to live (after enough shadow
/// data) or back to shadow.
pub async fn set_stage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service_type): Path<String>,
    Json(b): Json<StageBody>,
) -> HandlerResult {
    match state
        .noshow
        .set_stage(
            &service_type,
            &b.stage,
            &b.changed_by,
            &b.note,
            &ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/noshow/evaluation`: calibration, and before/after at each live clinic against the ones still in
/// shadow. Small cells withheld. Works while the score is off (and is empty then).
pub async fn evaluation(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> HandlerResult {
    match state.noshow.evaluation(q.days.unwrap_or(180), q.k).await {
        Ok(v) => ok(v),
        Err(e) => error_to_response(e),
    }
}

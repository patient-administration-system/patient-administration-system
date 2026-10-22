//! REST handlers for waiting-list validation campaigns (NEW-2). The rules are in
//! [`crate::list_validation`]: nothing here removes a patient without a named person's decision.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, LimitQuery, error_to_response, ok_json, ok_page, request_audit_ctx,
};
use crate::list_validation::NewCampaign;

#[derive(Debug, Deserialize)]
pub struct StartBody {
    pub name: String,
    pub started_by: String,
    #[serde(default)]
    pub service_type: Option<String>,
    #[serde(default)]
    pub min_weeks: Option<i64>,
    #[serde(default = "default_reply_days")]
    pub reply_in_days: i64,
    #[serde(default = "default_final_days")]
    pub final_wait_days: i64,
    #[serde(default = "default_cohort")]
    pub limit: i64,
}

fn default_reply_days() -> i64 {
    14
}
fn default_final_days() -> i64 {
    14
}
fn default_cohort() -> i64 {
    100
}

/// `POST /api/validation-campaigns`: take the longest-waiting pathways (not urgent, not already in a campaign,
/// not checked lately) and start asking whether the care is still needed.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns",
    tag = "Validation campaigns",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<StartBody>,
) -> HandlerResult {
    match state
        .list_validation
        .start(
            NewCampaign {
                name: b.name,
                started_by: b.started_by,
                service_type: b.service_type,
                min_weeks: b.min_weeks,
                reply_in_days: b.reply_in_days,
                final_wait_days: b.final_wait_days,
                limit: b.limit,
            },
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(c) => ok_json(c),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/validation-campaigns`: newest first, each with its item counts by state.
#[utoipa::path(
    get,
    path = "/api/validation-campaigns",
    tag = "Validation campaigns",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>, Query(q): Query<LimitQuery>) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let ids = match state
        .list_validation
        .campaigns(crate::paging::ask_for(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_to_response(e),
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match state.list_validation.campaign(id).await {
            Ok(c) => out.push(c),
            Err(e) => return error_to_response(e),
        }
    }
    ok_page(out, limit)
}

/// `GET /api/validation-campaigns/{id}`.
#[utoipa::path(
    get,
    path = "/api/validation-campaigns/{id}",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.list_validation.campaign(id).await {
        Ok(c) => ok_json(c),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ItemsQuery {
    pub state: Option<String>,
    /// `letter_due` or `decision_due`: what a person can act on now.
    pub next: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/validation-campaigns/{id}/items?state=&next=&limit=`.
#[utoipa::path(
    get,
    path = "/api/validation-campaigns/{id}/items",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn items(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ItemsQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .list_validation
        .items(
            id,
            q.state.as_deref(),
            q.next.as_deref(),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AskedBody {
    pub asked_by: String,
    /// `phone`, `sms`, `whatsapp`, `email` or `in_person`.
    pub channel: String,
}

/// `POST /api/validation-campaigns/{id}/items/{item}/asked`: a person has asked the patient.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns/{id}/items/{item}/asked",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`"), ("item" = String, Path, description = "Path parameter `item`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn asked(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, item)): Path<(Uuid, Uuid)>,
    Json(b): Json<AskedBody>,
) -> HandlerResult {
    match state
        .list_validation
        .mark_asked(
            id,
            item,
            &b.asked_by,
            &b.channel,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(i) => ok_json(i),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    /// `still_needed`, `no_longer_needed` or `unsure`.
    pub answer: String,
    pub recorded_by: String,
}

/// `POST /api/validation-campaigns/{id}/items/{item}/answer`: the patient's answer, recorded by a person.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns/{id}/items/{item}/answer",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`"), ("item" = String, Path, description = "Path parameter `item`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, item)): Path<(Uuid, Uuid)>,
    Json(b): Json<AnswerBody>,
) -> HandlerResult {
    match state
        .list_validation
        .record_answer(
            id,
            item,
            &b.answer,
            &b.recorded_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(i) => ok_json(i),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct LetterBody {
    pub sent_by: String,
}

/// `POST /api/validation-campaigns/{id}/items/{item}/letter-sent`: the letter has been posted to a patient who
/// did not reply by the campaign's date.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns/{id}/items/{item}/letter-sent",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`"), ("item" = String, Path, description = "Path parameter `item`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn letter_sent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, item)): Path<(Uuid, Uuid)>,
    Json(b): Json<LetterBody>,
) -> HandlerResult {
    match state
        .list_validation
        .letter_sent(id, item, &b.sent_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(i) => ok_json(i),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct DecisionBody {
    /// `keep` or `remove`.
    pub decision: String,
    pub decided_by: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/validation-campaigns/{id}/items/{item}/decision`: a named person keeps or removes the patient.
/// Refused until the patient said the care is no longer needed, or the wait for a reply to the letter has ended;
/// never a removal for a patient who still needs it or is unsure, or on an urgent service.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns/{id}/items/{item}/decision",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`"), ("item" = String, Path, description = "Path parameter `item`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, item)): Path<(Uuid, Uuid)>,
    Json(b): Json<DecisionBody>,
) -> HandlerResult {
    match state
        .list_validation
        .decide(
            id,
            item,
            &b.decision,
            &b.decided_by,
            b.note.as_deref(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(i) => ok_json(i),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReinstateBody {
    pub reinstated_by: String,
}

/// `POST /api/validation-campaigns/{id}/items/{item}/reinstate`: undo a removal within 90 days; the clock
/// continues as if the patient had never left.
#[utoipa::path(
    post,
    path = "/api/validation-campaigns/{id}/items/{item}/reinstate",
    tag = "Validation campaigns",
    params(("id" = String, Path, description = "Path parameter `id`"), ("item" = String, Path, description = "Path parameter `item`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn reinstate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, item)): Path<(Uuid, Uuid)>,
    Json(b): Json<ReinstateBody>,
) -> HandlerResult {
    match state
        .list_validation
        .reinstate(id, item, &b.reinstated_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(i) => ok_json(i),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// `GET /api/reports/validation-removals?from=&to=`: validation removals, reinstatements and keeps by service,
/// apart from treatment. No identifiers. Defaults to the last 90 days.
#[utoipa::path(
    get,
    path = "/api/reports/validation-removals",
    tag = "Reports",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn removals_report(
    State(state): State<AppState>,
    Query(q): Query<ReportQuery>,
) -> HandlerResult {
    let to = q.to.unwrap_or_else(|| Utc::now() + Duration::days(1));
    let from = q.from.unwrap_or_else(|| to - Duration::days(90));
    if from >= to || to - from > Duration::days(366) {
        return error_to_response(crate::Error::validation(
            "from must be before to, and the range at most a year",
        ));
    }
    match state.list_validation.report(from, to).await {
        Ok(v) => ok_json(serde_json::json!({ "from": from, "to": to, "by_service": v })),
        Err(e) => error_to_response(e),
    }
}

//! REST handlers for clinic session templates, slot types and the capacity report (NEW-8).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, request_audit_ctx,
};
use crate::clinic_templates::report::capacity_report;
use crate::clinic_templates::{Line, TemplateService, VersionInput};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_clinic_templates::CreateBody)]
pub struct CreateBody {
    pub schedule_id: Uuid,
    pub name: String,
    pub created_by: String,
}

/// `POST /api/session-templates`: start a template on a schedule. It makes no slots until a version is
/// proposed and approved by a second person.
#[utoipa::path(
    post,
    path = "/api/session-templates",
    tag = "Scheduling",
    request_body = CreateBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateBody>,
) -> HandlerResult {
    match TemplateService::new(state.db.clone())
        .create(
            b.schedule_id,
            &b.name,
            &b.created_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(id) => ok(serde_json::json!({ "id": id })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListQuery {
    pub schedule_id: Option<Uuid>,
}

/// `GET /api/session-templates?schedule_id=`: templates with every version and who proposed and approved it.
#[utoipa::path(
    get,
    path = "/api/session-templates",
    tag = "Scheduling",
    params(ListQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> HandlerResult {
    match TemplateService::new(state.db.clone())
        .list(q.schedule_id)
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ProposeBody {
    /// ISO weekday: 1 Monday to 7 Sunday.
    pub weekday: i16,
    /// `HH:MM` or `HH:MM:SS`, London local time.
    pub start_time: NaiveTime,
    pub slot_minutes: i32,
    pub effective_from: NaiveDate,
    pub lines: Vec<Line>,
    pub proposed_by: String,
}

/// `POST /api/session-templates/{id}/versions`: propose the next version. It generates nothing until a
/// different named person approves it.
#[utoipa::path(
    post,
    path = "/api/session-templates/{id}/versions",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ProposeBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn propose(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ProposeBody>,
) -> HandlerResult {
    let input = VersionInput {
        weekday: b.weekday,
        start_time: b.start_time,
        slot_minutes: b.slot_minutes,
        effective_from: b.effective_from,
        lines: b.lines,
        proposed_by: b.proposed_by,
    };
    match TemplateService::new(state.db.clone())
        .propose(id, &input, state.clock.now(), &request_audit_ctx(&headers))
        .await
    {
        Ok(version) => ok(serde_json::json!({ "id": id, "version": version })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_clinic_templates::ApproveBody)]
pub struct ApproveBody {
    pub approved_by: String,
}

/// `POST /api/session-templates/{id}/versions/{version}/approve`: approve a version. The approver must be
/// someone other than the proposer; once only.
#[utoipa::path(
    post,
    path = "/api/session-templates/{id}/versions/{version}/approve",
    tag = "Scheduling",
    params(
        ("id" = String, Path, description = "Path parameter `id`"),
        ("version" = String, Path, description = "Path parameter `version`"),
    ),
    request_body = ApproveBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn approve(
    State(state): State<AppState>,
    Path((id, version)): Path<(Uuid, i32)>,
    headers: HeaderMap,
    Json(b): Json<ApproveBody>,
) -> HandlerResult {
    match TemplateService::new(state.db.clone())
        .approve(
            id,
            version,
            &b.approved_by,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "version": version })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GenerateBody {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub generated_by: String,
}

/// `POST /api/session-templates/{id}/generate`: make the slots for each day in the range from the approved
/// version in force. Repeating it adds nothing.
#[utoipa::path(
    post,
    path = "/api/session-templates/{id}/generate",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = GenerateBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn generate(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<GenerateBody>,
) -> HandlerResult {
    match TemplateService::new(state.db.clone())
        .generate(
            id,
            b.from,
            b.to,
            &b.generated_by,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(g) => ok_json(g),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RingFenceBody {
    pub ring_fenced: bool,
    pub set_by: String,
}

/// `PUT /api/slots/{id}/ring-fence`: hold a free slot out of automatic offers (first available, rebooking,
/// IHE `$find`), or lift the hold. A person can still book it by choosing it.
#[utoipa::path(
    put,
    path = "/api/slots/{id}/ring-fence",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = RingFenceBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn ring_fence(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<RingFenceBody>,
) -> HandlerResult {
    match TemplateService::new(state.db.clone())
        .set_ring_fence(id, b.ring_fenced, &b.set_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok(serde_json::json!({ "slot_id": id, "ring_fenced": b.ring_fenced })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ReportQuery {
    pub service_type: String,
    /// Start of the forward window; defaults to now.
    pub from: Option<DateTime<Utc>>,
    /// End of the forward window; defaults to 28 days after `from`.
    pub to: Option<DateTime<Utc>>,
    /// How far back demand and non-attendance are measured; defaults to 84 days.
    pub lookback_days: Option<i64>,
}

/// `GET /api/capacity/report?service_type=`: slots by type in a forward window against required slots per
/// week from recent referrals and non-attendance. Plain arithmetic stated in the response, not NHS England's
/// core model.
#[utoipa::path(
    get,
    path = "/api/capacity/report",
    tag = "Scheduling",
    params(ReportQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn report(State(state): State<AppState>, Query(q): Query<ReportQuery>) -> HandlerResult {
    let from = q.from.unwrap_or_else(|| state.clock.now());
    let to = q.to.unwrap_or(from + Duration::days(28));
    match capacity_report(
        &state.db,
        &q.service_type,
        from,
        to,
        q.lookback_days.unwrap_or(84),
    )
    .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

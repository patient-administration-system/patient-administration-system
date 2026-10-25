//! REST handlers for referrals (CORE-1).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::ok_json;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page, request_audit_ctx};
use crate::models::waitlist::Priority;
use crate::referrals::{Created, NewReferral};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_referrals::CreateBody)]
pub struct CreateBody {
    pub person_id: Uuid,
    /// `gp`, `e-rs` or `internal`.
    pub source: String,
    /// The sender's own reference, for example an e-RS UBRN. Required for `e-rs`.
    #[serde(default)]
    pub external_ref: Option<String>,
    pub target_service: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub referring_worker_id: Option<Uuid>,
    #[serde(default)]
    pub received_at: Option<DateTime<Utc>>,
}

/// `POST /api/referrals`: receive a referral. The same `(source, external_ref)` twice is one referral
/// (`duplicate: true`).
#[utoipa::path(
    post,
    path = "/api/referrals",
    tag = "Referrals",
    request_body = CreateBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Created>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateBody>,
) -> HandlerResult {
    match state
        .referrals
        .create(
            NewReferral {
                person_id: b.person_id,
                source: b.source,
                external_ref: b.external_ref,
                target_service: b.target_service,
                reason: b.reason,
                referring_worker_id: b.referring_worker_id,
                received_at: b.received_at,
            },
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok((referral, duplicate)) => ok_json(Created {
            referral,
            duplicate,
        }),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListQuery {
    pub status: Option<String>,
    pub service: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/referrals?status=&service=`: oldest first.
#[utoipa::path(
    get,
    path = "/api/referrals",
    tag = "Referrals",
    params(ListQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::referrals::Referral>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .referrals
        .list(
            q.status.as_deref(),
            q.service.as_deref(),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(v) => ok_page(v, limit),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/referrals/{id}`.
#[utoipa::path(
    get,
    path = "/api/referrals/{id}",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Referral>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.referrals.get(id).await {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/referrals/{id}/book-first-available`: book the earliest free slot of the target service and
/// link the appointment. With none free, 409 and the referral stays open.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/book-first-available",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Booked>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn book_first_available(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state
        .referrals
        .book_first_available(id, &request_audit_ctx(&headers))
        .await
    {
        Ok(b) => ok_json(b),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct WaitlistBody {
    #[serde(default)]
    pub priority: Option<Priority>,
}

/// `POST /api/referrals/{id}/waitlist`: put the patient on the waiting list for the service.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/waitlist",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = WaitlistBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Referral>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn waitlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<WaitlistBody>>,
) -> HandlerResult {
    let p = body.and_then(|b| b.0.priority).unwrap_or(Priority::Routine);
    match state
        .referrals
        .waitlist(id, p, &request_audit_ctx(&headers))
        .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_referrals::CloseBody)]
pub struct CloseBody {
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /api/referrals/{id}/close`.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/close",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = CloseBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Referral>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<CloseBody>>,
) -> HandlerResult {
    let reason = body.and_then(|b| b.0.reason);
    match state
        .referrals
        .close(id, reason.as_deref(), &request_audit_ctx(&headers))
        .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_referrals::TriageBody)]
pub struct TriageBody {
    /// `returned_with_advice` or `redirected`.
    pub outcome: String,
    /// The named person (a consultant or their delegate) who triaged it.
    pub triaged_by: String,
    /// For a redirect: the service the patient goes to.
    #[serde(default)]
    pub redirect_to_service: Option<String>,
}

/// `POST /api/referrals/{id}/triage`: a consultant returns the referral with advice or redirects it to
/// another service (a redirect creates the new referral). Only a referral just received can be triaged.
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/triage",
    tag = "Referrals",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = TriageBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::referrals::Referral>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn triage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<TriageBody>,
) -> HandlerResult {
    match state
        .referrals
        .triage(
            id,
            &b.outcome,
            &b.triaged_by,
            b.redirect_to_service.as_deref(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

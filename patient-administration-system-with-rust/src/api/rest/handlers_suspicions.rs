//! REST handlers for suspected conditions (WPAS-1, spec 20, FR-207 to FR-216).

use axa::extract::{Json, Path, Query};
use axum::{extract::State, http::HeaderMap};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::ok_json;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::suspicions::NewSuspicion;

/// `GET /api/suspicion-profile`: the active profile, which conditions are tracked and each one's subtypes,
/// target, cancel rule and booking priority (WPAS-8, FR-221).
#[utoipa::path(
    get,
    path = "/api/suspicion-profile",
    tag = "Suspected conditions",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::suspicions::Profile>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn profile(State(state): State<AppState>) -> HandlerResult {
    ok_json(state.suspicions.profile())
}

/// `POST /api/referrals/{id}/suspicions`: record that a named clinician suspects a condition (FR-207).
#[utoipa::path(
    post,
    path = "/api/referrals/{id}/suspicions",
    tag = "Suspected conditions",
    params(("id" = String, Path, description = "The referral")),
    request_body = NewSuspicion,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::suspicions::Suspicion>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn record(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<NewSuspicion>,
) -> HandlerResult {
    match state
        .suspicions
        .record(id, b, &request_audit_ctx(&headers))
        .await
    {
        Ok(s) => ok_json(s),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/referrals/{id}/suspicions`: every suspicion on a referral, oldest point of suspicion first.
#[utoipa::path(
    get,
    path = "/api/referrals/{id}/suspicions",
    tag = "Suspected conditions",
    params(("id" = String, Path, description = "The referral")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::suspicions::Suspicion>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn for_referral(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.suspicions.for_referral(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/suspicions/{id}`: one suspicion, with its version as `ETag`.
#[utoipa::path(
    get,
    path = "/api/suspicions/{id}",
    tag = "Suspected conditions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::suspicions::Suspicion>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.suspicions.get(id).await {
        Ok(s) => ok_json(s),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_suspicions::StopBody)]
pub struct StopBody {
    #[schema(value_type = crate::suspicions::StopReason)]
    pub reason: String,
    /// The date the clock stopped: the treatment start, the downgrade, or the death.
    pub stopped_on: NaiveDate,
    pub stopped_by: String,
}

/// `POST /api/suspicions/{id}/stop`: stop the clock, once, for one reason (FR-209).
/// Needs `If-Match` with the suspicion's version, the `ETag` of `GET /api/suspicions/{id}`: none is `428`, a stale one `412` (FR-216).
#[utoipa::path(
    post,
    path = "/api/suspicions/{id}/stop",
    tag = "Suspected conditions",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = StopBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::suspicions::Suspicion>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<StopBody>,
) -> HandlerResult {
    let expected =
        match crate::api::rest::concurrency::require_if_match(&headers, "suspected condition") {
            Ok(t) => t,
            Err(e) => return error_to_response(e),
        };
    match state
        .suspicions
        .stop_if_unchanged(
            id,
            &b.reason,
            b.stopped_on,
            &b.stopped_by,
            expected,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(s) => ok_json(s),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct OpenQuery {
    /// Only this condition (a code of the active profile).
    pub condition: Option<String>,
    /// Only this subtype.
    pub subtype: Option<String>,
}

/// `GET /api/suspicions?condition&subtype&limit`: the open list, longest wait first (FR-212). Paged.
#[utoipa::path(
    get,
    path = "/api/suspicions",
    tag = "Suspected conditions",
    params(crate::paging::PageQuery, OpenQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::suspicions::Suspicion>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn open(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Query(q): Query<OpenQuery>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 100) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    // A condition the active profile does not define is a mistake, not an empty list.
    if let Some(c) = q.condition.as_deref()
        && state.suspicions.profile().condition(c).is_none()
    {
        return error_to_response(crate::Error::validation(format!(
            "condition must be one of {}",
            state.suspicions.profile().condition_codes().join(", ")
        )));
    }
    match state
        .suspicions
        .open(q.condition.as_deref(), q.subtype.as_deref(), page.rows())
        .await
    {
        Ok(v) => crate::api::rest::handlers::ok_cursor_page(&page, v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ReportQuery {
    /// First stop date counted (inclusive).
    pub from: NaiveDate,
    /// Last stop date counted (inclusive).
    pub to: NaiveDate,
}

/// `GET /api/suspicions/report?from&to`: suspicions whose clock stopped in the period, per condition and tumour
/// site, with the share within target (FR-213).
#[utoipa::path(
    get,
    path = "/api/suspicions/report",
    tag = "Suspected conditions",
    params(ReportQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::suspicions::SuspicionReport>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn report(State(state): State<AppState>, Query(q): Query<ReportQuery>) -> HandlerResult {
    match state.suspicions.report(q.from, q.to).await {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

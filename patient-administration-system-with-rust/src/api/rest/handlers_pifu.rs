//! REST handlers for patient-initiated follow-up (NEW-3).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::analytics::AnalyticsService;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, ok_page, request_audit_ctx,
};
use crate::pifu::{Decision, NewEpisode, PifuService};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OpenBody {
    pub service_type: String,
    /// The conditions, in words the patient can follow (10 to 1000 characters).
    pub conditions: String,
    /// When the window ends (at most two years ahead).
    pub expires_at: DateTime<Utc>,
    pub opened_by: String,
    /// Needed when the service is on the urgent follow-up list: the clinician who says PIFU is safe for it.
    pub urgent_override_by: Option<String>,
}

/// `POST /api/patients/{id}/pifu`: a clinician opens a PIFU episode. Audited.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/pifu",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = OpenBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn open(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<OpenBody>,
) -> HandlerResult {
    match PifuService::new(state.db.clone())
        .open(
            id,
            &NewEpisode {
                service_type: &b.service_type,
                conditions: &b.conditions,
                expires_at: b.expires_at,
                opened_by: &b.opened_by,
                urgent_override_by: b.urgent_override_by.as_deref(),
            },
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RequestBody {
    /// `staff`, `phone`, `text`, `reply_link` or `portal`.
    pub channel: String,
}

/// `POST /api/pifu/{id}/request`: the patient asks to be seen. Inside the window the request is a booking
/// task (`route: booking`); after it, a person decides (`route: person`).
#[utoipa::path(
    post,
    path = "/api/pifu/{id}/request",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = RequestBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn request(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<RequestBody>,
) -> HandlerResult {
    match PifuService::new(state.db.clone())
        .request(
            id,
            &b.channel,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RequestsQuery {
    /// `booking` (default) or `person`.
    pub route: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/pifu/requests?route=`: open requests, oldest first, for the booking team or for a person's
/// decision. Paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/pifu/requests",
    tag = "Scheduling",
    params(RequestsQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn requests(
    State(state): State<AppState>,
    Query(q): Query<RequestsQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match PifuService::new(state.db.clone())
        .open_requests(
            q.route.as_deref().unwrap_or("booking"),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct HandleBody {
    pub handled_by: String,
    /// The appointment booked for the request, when there is one.
    pub appointment_id: Option<Uuid>,
}

/// `POST /api/pifu/requests/{id}/handle`: a named person has dealt with the request. Once only; audited.
#[utoipa::path(
    post,
    path = "/api/pifu/requests/{id}/handle",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = HandleBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<HandleBody>,
) -> HandlerResult {
    match PifuService::new(state.db.clone())
        .handle(
            id,
            &b.handled_by,
            b.appointment_id,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "status": "handled" })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ReviewsQuery {
    pub limit: Option<i64>,
}

/// `GET /api/pifu/due-reviews`: episodes whose window has ended and that nobody has reviewed. The review
/// task: a person discharges or extends; nothing does it for them. Paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/pifu/due-reviews",
    tag = "Scheduling",
    params(ReviewsQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn due_reviews(
    State(state): State<AppState>,
    Query(q): Query<ReviewsQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match PifuService::new(state.db.clone())
        .due_reviews(state.clock.now(), crate::paging::ask_for(limit))
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_pifu::ReviewBody)]
pub struct ReviewBody {
    /// `discharge` or `extend`.
    pub decision: String,
    /// For `extend`: the new end of the window.
    pub expires_at: Option<DateTime<Utc>>,
    pub reviewed_by: String,
}

/// `POST /api/pifu/{id}/review`: a named person discharges or extends an episode whose window has ended.
#[utoipa::path(
    post,
    path = "/api/pifu/{id}/review",
    tag = "Scheduling",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ReviewBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn review(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ReviewBody>,
) -> HandlerResult {
    let decision = match (b.decision.as_str(), b.expires_at) {
        ("discharge", None) => Decision::Discharge,
        ("extend", Some(until)) => Decision::Extend(until),
        ("discharge", Some(_)) => {
            return error_to_response(crate::Error::validation("a discharge takes no expires_at"));
        }
        ("extend", None) => {
            return error_to_response(crate::Error::validation("an extension needs expires_at"));
        }
        _ => {
            return error_to_response(crate::Error::validation(
                "decision must be discharge or extend",
            ));
        }
    };
    match PifuService::new(state.db.clone())
        .review(
            id,
            decision,
            &b.reviewed_by,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "decision": b.decision })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct UptakeQuery {
    /// The smallest cell shown; a caller may raise it above the configured minimum, never lower it.
    pub k: Option<i64>,
}

/// `GET /api/reports/pifu`: PIFU uptake apart from discharges. Counts only; small cells withheld.
#[utoipa::path(
    get,
    path = "/api/reports/pifu",
    tag = "Reports",
    params(UptakeQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn uptake(State(state): State<AppState>, Query(q): Query<UptakeQuery>) -> HandlerResult {
    let k = q.k.unwrap_or(0).max(AnalyticsService::minimum_k());
    match PifuService::new(state.db.clone()).uptake(k).await {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

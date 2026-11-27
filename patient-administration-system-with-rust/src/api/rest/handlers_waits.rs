//! REST handlers for wait-time and lead-time data (CAP-8).

use axa::extract::{Json, Path, Query};
use axum::{extract::State, http::HeaderMap};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, request_audit_ctx};
use crate::api::rest::handlers::{ok, ok_json};
use crate::paging;

/// `GET /api/appointments/{id}/lead-time`: booking to visit, in whole days.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/lead-time",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::waits::PatientWaits>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn patient_waits(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.waits.patient_waits(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DueQuery {
    pub weeks: Option<i64>,
    pub repeat_days: Option<i64>,
}

/// One long waiter nobody has asked recently whether they still need the appointment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct DueCheck {
    pub pathway_id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub weeks_waited: u32,
    pub last_check_at: Option<jiff::Timestamp>,
}

/// `GET /api/pathways/due-check?weeks=26&repeat_days=90`: long waiters nobody has asked recently whether
/// they still need this, earliest clock start first (ties by pathway). A page is filled from the open pathways
/// a window at a time, keeping those over the threshold, and reads at most ten pages' worth per request; a page
/// that spent that budget may be short, with a next page from where the reading stopped.
#[utoipa::path(
    get,
    path = "/api/pathways/due-check",
    tag = "Pathways",
    params(crate::paging::PageQuery, DueQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<DueCheck>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn due_for_check(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Query(q): Query<DueQuery>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, paging::MAX_LIMIT) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    let weeks = match crate::waits::WaitsService::check_weeks(q.weeks) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    // Read the open pathways a window at a time, keep those over the threshold, and stop when the page is full
    // or ten pages' worth have been read: the page is never short for the pathways under the threshold, and a
    // request never reads the whole table (axa-cursor-pagination `fill`).
    // At most 500 rows a page, so ten pages always fit; the fallback only names the ceiling.
    let budget = u64::try_from(page.limit())
        .unwrap_or(0)
        .checked_mul(10)
        .unwrap_or(u64::MAX);
    let filled = page
        .fill(
            budget,
            |rows| state.waits.unchecked_window(q.repeat_days, rows),
            |(_, p)| i64::from(p.weeks_waited) >= weeks,
        )
        .await;
    match filled {
        Ok((rows, meta, link)) => {
            let rows: Vec<DueCheck> = rows
                .into_iter()
                .map(|(person_id, p)| DueCheck {
                    pathway_id: p.pathway_id,
                    person_id,
                    target_service: p.target_service,
                    weeks_waited: p.weeks_waited,
                    last_check_at: p.last_check_at,
                })
                .collect();
            let mut response = crate::api::rest::handlers::ok_json_page(rows, meta);
            if let Some(link) = link {
                response
                    .headers_mut()
                    .insert(axum::http::header::LINK, link);
            }
            response
        }
        Err(e) => error_to_response(e),
    }
}

/// The id of the check just asked.
#[derive(Debug, serde::Serialize, Deserialize, utoipa::ToSchema)]
pub struct Asked {
    pub id: Uuid,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AskBody {
    pub asked_by: String,
}

/// `POST /api/pathways/{id}/still-needed-checks`: a person asks the patient whether they still need this.
#[utoipa::path(
    post,
    path = "/api/pathways/{id}/still-needed-checks",
    tag = "Pathways",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = AskBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Asked>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        Ok(check) => ok_json(Asked { id: check }),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_waits::AnswerBody)]
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
    request_body = AnswerBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::api::Nothing>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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

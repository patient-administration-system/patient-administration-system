//! REST handlers for a triage category recorded on an emergency encounter (NEW-22).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok, request_audit_ctx};
use crate::triage::TriageService;
use crate::triage::targets::Targets;
use crate::{Error, Result};
use serde::Serialize;

/// Serialize or answer with an internal error, never a `null` (QUAL-27).
fn json<T: Serialize>(value: Result<T>) -> HandlerResult {
    match value.and_then(|v| {
        serde_json::to_value(v)
            .map_err(|e| Error::internal(format!("response could not be serialized: {e}")))
    }) {
        Ok(v) => ok(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_triage::TriageBody)]
pub struct TriageBody {
    /// The triage system; `mts` (Manchester Triage System).
    pub triage_system: String,
    /// The edition or version of the system, for example "3.8".
    pub system_version: Option<String>,
    /// `immediate`, `very_urgent`, `urgent`, `standard` or `non_urgent`.
    pub category: String,
    /// The presentational flowchart used, as free text.
    pub flowchart: Option<String>,
    /// The person who assigned the category.
    pub assessed_by: String,
    /// When it was assigned; defaults to now, never in the future.
    pub assessed_at: Option<DateTime<Utc>>,
}

/// `POST /api/encounters/{id}/triage`: record a triage category assigned elsewhere. The PAS records it and
/// never computes it. Audited; a re-triage adds a row and keeps the history.
#[utoipa::path(
    post,
    path = "/api/encounters/{id}/triage",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = TriageBody,
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
    Json(b): Json<TriageBody>,
) -> HandlerResult {
    let now = state.clock.now();
    json(
        TriageService::new(state.db.clone())
            .record(
                id,
                &b.triage_system,
                b.system_version.as_deref(),
                &b.category,
                b.flowchart.as_deref(),
                &b.assessed_by,
                b.assessed_at.unwrap_or(now),
                now,
                &request_audit_ctx(&headers),
            )
            .await,
    )
}

/// `GET /api/encounters/{id}/triage`: every assessment of the encounter, oldest first; the last is current.
#[utoipa::path(
    get,
    path = "/api/encounters/{id}/triage",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    json(TriageService::new(state.db.clone()).history(id).await)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FirstContactBody {
    /// The clinician who first saw the patient.
    pub seen_by: String,
    /// When; defaults to now, not before arrival and never in the future.
    pub seen_at: Option<DateTime<Utc>>,
}

/// `POST /api/encounters/{id}/first-contact`: record, once, when a clinician first saw the patient. Audited.
#[utoipa::path(
    post,
    path = "/api/encounters/{id}/first-contact",
    tag = "Encounters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = FirstContactBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn first_contact(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<FirstContactBody>,
) -> HandlerResult {
    let now = state.clock.now();
    json(
        TriageService::new(state.db.clone())
            .record_first_contact(
                id,
                &b.seen_by,
                b.seen_at.unwrap_or(now),
                now,
                &request_audit_ctx(&headers),
            )
            .await
            .map(|()| serde_json::json!({ "encounter_id": id })),
    )
}

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    /// Start of the window (inclusive); defaults to 24 hours before `to`.
    pub from: Option<DateTime<Utc>>,
    /// End of the window (exclusive); defaults to now.
    pub to: Option<DateTime<Utc>>,
}

/// `GET /api/triage/waits`: attendances in the window against each category's target (`PAS_TRIAGE_TARGETS`),
/// with those past target listed. Attendances with no category are counted as excluded, not guessed.
#[utoipa::path(
    get,
    path = "/api/triage/waits",
    tag = "Encounters",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn waits(State(state): State<AppState>, Query(q): Query<WindowQuery>) -> HandlerResult {
    let now = state.clock.now();
    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(to - chrono::Duration::hours(24));
    json(
        TriageService::new(state.db.clone())
            .waits_report(from, to, &Targets::from_env(), now)
            .await,
    )
}

/// `GET /api/triage/queue`: who has arrived and not yet been seen, by category then arrival. A display only:
/// the order of care is a clinician's decision.
#[utoipa::path(
    get,
    path = "/api/triage/queue",
    tag = "Encounters",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn queue(State(state): State<AppState>) -> HandlerResult {
    json(
        TriageService::new(state.db.clone())
            .queue(&Targets::from_env(), state.clock.now())
            .await,
    )
}

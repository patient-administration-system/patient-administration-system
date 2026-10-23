//! REST handlers for a triage category recorded on an emergency encounter (NEW-22).

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok, request_audit_ctx};
use crate::triage::TriageService;
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

#[derive(Debug, Deserialize)]
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

//! REST handlers for the privacy-safe analytics feed (CAP-11).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::analytics::Dim;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;

#[derive(Debug, Deserialize)]
pub struct OutcomesQuery {
    /// Comma-separated: specialty, language, age_band, deprivation_quintile, time_of_day, outcome, notice.
    pub group_by: String,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// A larger `k` than the configured minimum; a smaller one is ignored.
    pub k: Option<i64>,
    /// `csv` for CSV; JSON otherwise.
    pub format: Option<String>,
}

/// `GET /api/analytics/appointment-outcomes`: counts of appointments by the chosen dimensions. No identifier
/// appears anywhere in it, and cells under `k` are withheld (never shown as zero).
pub async fn outcomes(State(state): State<AppState>, Query(q): Query<OutcomesQuery>) -> Response {
    let mut dims = Vec::new();
    for name in q
        .group_by
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match Dim::parse(name) {
            Some(d) => dims.push(d),
            None => {
                return error_to_response(Error::validation(format!("unknown dimension {name}")))
                    .into_response();
            }
        }
    }
    match state.analytics.outcomes(&dims, q.from, q.to, q.k).await {
        Ok(r) if q.format.as_deref() == Some("csv") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            r.to_csv(),
        )
            .into_response(),
        Ok(r) => (
            StatusCode::OK,
            Json(ApiResponse::ok(
                serde_json::to_value(r).unwrap_or(Value::Null),
            )),
        )
            .into_response(),
        Err(e) => error_to_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct DemographicsBody {
    #[serde(default)]
    pub age_band: Option<String>,
    #[serde(default)]
    pub deprivation_quintile: Option<i16>,
    pub source: String,
}

/// `PUT /api/analytics/demographics/{person_id}`: an upstream job pushes the patient's coarse bands.
pub async fn set_demographics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(person_id): Path<Uuid>,
    Json(b): Json<DemographicsBody>,
) -> HandlerResult {
    let uc = user_context_from_headers(&headers);
    let ctx = RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    match state
        .analytics
        .set_demographics(
            person_id,
            b.age_band.as_deref(),
            b.deprivation_quintile,
            &b.source,
            &ctx,
        )
        .await
    {
        Ok(()) => (StatusCode::OK, Json(ApiResponse::ok(Value::Null))),
        Err(e) => error_to_response(e),
    }
}

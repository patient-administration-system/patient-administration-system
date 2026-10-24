//! REST handlers for activity and performance reporting (CORE-7).

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{error_to_response, ok_json};
use crate::reports::GroupBy;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ActivityQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// `day` (default) or `schedule`.
    pub group_by: Option<String>,
    /// `csv` for CSV; JSON otherwise.
    pub format: Option<String>,
}

/// `GET /api/reports/clinic-activity`: appointments by outcome, utilization, lead time and waiting-time
/// statistics, per day or per schedule. Counts reconcile; an empty range gives zeros. No identifiers.
#[utoipa::path(
    get,
    path = "/api/reports/clinic-activity",
    tag = "Reports",
    params(ActivityQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn clinic_activity(
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> Response {
    let group = match q.group_by.as_deref() {
        None => GroupBy::Day,
        Some(g) => match GroupBy::parse(g) {
            Some(g) => g,
            None => {
                return error_to_response(Error::validation("group_by must be day or schedule"))
                    .into_response();
            }
        },
    };
    match state.reports.clinic_activity(q.from, q.to, group).await {
        Ok(r) if q.format.as_deref() == Some("csv") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            r.to_csv(),
        )
            .into_response(),
        Ok(r) => ok_json(r).into_response(),
        Err(e) => error_to_response(e).into_response(),
    }
}

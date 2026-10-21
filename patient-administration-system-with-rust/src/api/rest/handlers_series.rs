//! REST handlers for appointment series.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::models::appointment::CancellationReason;
use crate::models::appointment_series::RecurrenceRule;
use crate::scheduling::series::CreateSeriesInput;

#[derive(Debug, serde::Deserialize)]
pub struct CreateSeriesBody {
    pub person_id: Uuid,
    #[serde(default)]
    pub worker_id: Option<Uuid>,
    pub service_type: String,
    pub start_datetime: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: u32,
    pub rule: RecurrenceRule,
    #[serde(default)]
    pub reason: Option<String>,
}

impl From<CreateSeriesBody> for CreateSeriesInput {
    fn from(b: CreateSeriesBody) -> Self {
        Self {
            person_id: b.person_id,
            worker_id: b.worker_id,
            service_type: b.service_type,
            start_datetime: b.start_datetime,
            duration_minutes: b.duration_minutes,
            rule: b.rule,
            reason: b.reason,
        }
    }
}

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `POST /api/appointment-series` — create a recurring appointment plan
/// and materialize its occurrences (atomically). Body: `CreateSeriesInput`.
#[utoipa::path(
    post,
    path = "/api/appointment-series",
    tag = "Appointment series",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSeriesBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.series.create(body.into(), &ctx).await {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointment-series/:id/cancel` — cancel the series and
/// every non-terminal occurrence.
#[utoipa::path(
    post,
    path = "/api/appointment-series/{id}/cancel",
    tag = "Appointment series",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_series(
    State(state): State<AppState>,
    Path(series_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .series
        .cancel(series_id, CancellationReason::PatientRequest, &ctx)
        .await
    {
        Ok(s) => ok(serde_json::to_value(s).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/appointment-series/:id` — full series with occurrence list.
#[utoipa::path(
    get,
    path = "/api/appointment-series/{id}",
    tag = "Appointment series",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_series(
    State(state): State<AppState>,
    Path(series_id): Path<Uuid>,
) -> HandlerResult {
    match state.series.get_with_occurrences(series_id).await {
        Ok(Some(r)) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("appointment_series {series_id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/:id/appointment-series` — series for a person.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/appointment-series",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_series_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match state.series.list_by_patient(person_id).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

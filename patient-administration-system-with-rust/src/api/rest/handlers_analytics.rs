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

fn audit_ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

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
#[utoipa::path(
    get,
    path = "/api/analytics/appointment-outcomes",
    tag = "Analytics",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn outcomes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<OutcomesQuery>,
) -> Response {
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
    if let Err(e) = state
        .analytics
        .charge_query(
            "appointment-outcomes",
            serde_json::json!({ "group_by": q.group_by, "from": q.from, "to": q.to, "k": q.k }),
            &audit_ctx(&headers),
        )
        .await
    {
        return error_to_response(e).into_response();
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

#[derive(Debug, Deserialize)]
pub struct RefillQuery {
    /// Comma-separated: notice_band, release_class, specialty, language, age_band, deprivation_quintile.
    pub group_by: String,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub k: Option<i64>,
    pub format: Option<String>,
}

/// `GET /api/analytics/refill-rates`: how often released slots are refilled, by notice period, early-release
/// share and the chosen splits (deprivation quintile, language, ...). Same rules as the outcomes feed: no
/// identifier, cells under `k` withheld, and nothing in it feeds back into what any patient is offered.
#[utoipa::path(
    get,
    path = "/api/analytics/refill-rates",
    tag = "Analytics",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn refill_rates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RefillQuery>,
) -> Response {
    let mut dims = Vec::new();
    for name in q
        .group_by
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match crate::analytics::RefillDim::parse(name) {
            Some(d) => dims.push(d),
            None => {
                return error_to_response(Error::validation(format!("unknown dimension {name}")))
                    .into_response();
            }
        }
    }
    if let Err(e) = state
        .analytics
        .charge_query(
            "refill-rates",
            serde_json::json!({ "group_by": q.group_by, "from": q.from, "to": q.to, "k": q.k }),
            &audit_ctx(&headers),
        )
        .await
    {
        return error_to_response(e).into_response();
    }
    match state.analytics.refill_rates(&dims, q.from, q.to, q.k).await {
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

/// `PUT /api/analytics/demographics/{person_id}`: an upstream job pushes the patient's coarse bands.
#[utoipa::path(
    put,
    path = "/api/analytics/demographics/{person_id}",
    tag = "Analytics",
    params(("person_id" = String, Path, description = "Path parameter `person_id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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

#[derive(Debug, Deserialize)]
pub struct LookupQuery {
    /// Which dataset the file was built from, for example `WIMD 2019 + ONSPD Aug 2026`.
    pub dataset: String,
    pub loaded_by: String,
}

/// `PUT /api/analytics/deprivation-lookup?dataset=...&loaded_by=...`: the health board's postcode to quintile
/// file as CSV (`postcode,quintile`). The PAS ships no data.
#[utoipa::path(
    put,
    path = "/api/analytics/deprivation-lookup",
    tag = "Analytics",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn load_deprivation_lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LookupQuery>,
    body: String,
) -> HandlerResult {
    let uc = user_context_from_headers(&headers);
    let ctx = RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    };
    match state
        .analytics
        .load_deprivation_lookup(&body, &q.dataset, &q.loaded_by, &ctx)
        .await
    {
        Ok((loaded, rejected)) => (
            StatusCode::OK,
            Json(ApiResponse::ok(
                serde_json::json!({ "loaded": loaded, "rejected": rejected }),
            )),
        ),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RefreshQuery {
    pub limit: Option<i64>,
}

/// `POST /api/analytics/demographics/refresh?limit=`: fill the bands for people who have none, from the
/// person record and the lookup. Idempotent; run it daily.
#[utoipa::path(
    post,
    path = "/api/analytics/demographics/refresh",
    tag = "Analytics",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn refresh_demographics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RefreshQuery>,
) -> HandlerResult {
    let Some(people) = state.person_client.clone() else {
        return error_to_response(Error::upstream(
            "the bands come from person-service, which is not configured",
        ));
    };
    let uc = user_context_from_headers(&headers);
    let ctx = RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    };
    match state
        .analytics
        .refresh_demographics(people.as_ref(), q.limit.unwrap_or(500), &uc, &ctx)
        .await
    {
        Ok(n) => (
            StatusCode::OK,
            Json(ApiResponse::ok(serde_json::json!({ "updated": n }))),
        ),
        Err(e) => error_to_response(e),
    }
}

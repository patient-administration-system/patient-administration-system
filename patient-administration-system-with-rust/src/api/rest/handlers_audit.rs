//! REST handlers for audit query.

use axum::extract::{Path, Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::ok_json;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::db::repositories::audit::AuditLogRepository;

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit")]
    pub limit: u64,
}

#[derive(Debug, Deserialize)]
pub struct EntityQuery {
    pub entity_type: String,
    pub entity_id: Uuid,
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_limit() -> u64 {
    50
}

const MAX_LIMIT: u64 = 500;

/// `GET /api/audit/recent?limit=N`.
#[utoipa::path(
    get,
    path = "/api/audit/recent",
    tag = "Audit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_recent(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = q.limit.min(MAX_LIMIT);
    match AuditLogRepository::list_recent(&state.db, limit).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/audit/entity?entity_type=…&entity_id=…&limit=N`.
#[utoipa::path(
    get,
    path = "/api/audit/entity",
    tag = "Audit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_for_entity(
    State(state): State<AppState>,
    Query(q): Query<EntityQuery>,
) -> HandlerResult {
    let limit = q.limit.min(MAX_LIMIT);
    match AuditLogRepository::list_for_entity(&state.db, &q.entity_type, q.entity_id, limit).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/:id/audit/local` — PAS-local audit rows whose
/// `entity_id == person_id`. Useful to inspect just the PAS-owned
/// actions taken against the person (admit, charge, letter, etc.). The
/// proxy variant `/api/patients/:id/audit` (in `upstream_proxy`) hits
/// the upstream service for demographic-mutation audit.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/audit/local",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_local_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = q.limit.min(MAX_LIMIT);
    match AuditLogRepository::list_for_entity(&state.db, "patient", person_id, limit).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

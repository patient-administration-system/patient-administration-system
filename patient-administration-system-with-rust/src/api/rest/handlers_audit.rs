//! REST handlers for audit query.

use axum::extract::{Path, Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page};
use crate::db::repositories::audit::AuditLogRepository;

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct EntityQuery {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub limit: Option<i64>,
}

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
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let fetch = u64::try_from(crate::paging::ask_for(limit)).unwrap_or(0);
    match AuditLogRepository::list_recent(&state.db, fetch).await {
        Ok(rows) => ok_page(rows, limit),
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
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let fetch = u64::try_from(crate::paging::ask_for(limit)).unwrap_or(0);
    match AuditLogRepository::list_for_entity(&state.db, &q.entity_type, q.entity_id, fetch).await {
        Ok(rows) => ok_page(rows, limit),
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
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let fetch = u64::try_from(crate::paging::ask_for(limit)).unwrap_or(0);
    match AuditLogRepository::list_for_entity(&state.db, "patient", person_id, fetch).await {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct BalpQuery {
    /// Start of the window (inclusive); defaults to 24 hours before `to`.
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// End of the window (exclusive); defaults to now.
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    /// At most 500; default 100.
    pub limit: Option<i64>,
}

/// `GET /api/audit/balp`: audit rows in a window as FHIR **R4** `AuditEvent`s in the shape of IHE Basic Audit
/// Log Patterns, oldest first, as a searchset Bundle. An event names a BALP profile only when it meets that
/// profile's structural constraints (`audit_export::violations`). The export is itself audited.
#[utoipa::path(
    get,
    path = "/api/audit/balp",
    tag = "Audit",
    responses(
        (status = 200, description = "A FHIR R4 Bundle of AuditEvent (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn export_balp(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<BalpQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

    use crate::db::entities::audit_log;

    let now = state.clock.now();
    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(to - chrono::Duration::hours(24));
    if to <= from {
        return error_to_response(crate::Error::validation("to must be after from"))
            .into_response();
    }
    if to - from > chrono::Duration::days(31) {
        return error_to_response(crate::Error::validation(
            "the window can be at most 31 days",
        ))
        .into_response();
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 500) as u64;
    let mut find = audit_log::Entity::find()
        .filter(audit_log::Column::At.gte(from))
        .filter(audit_log::Column::At.lt(to));
    if let Some(t) = &q.entity_type {
        find = find.filter(audit_log::Column::EntityType.eq(t.as_str()));
    }
    if let Some(id) = q.entity_id {
        find = find.filter(audit_log::Column::EntityId.eq(id));
    }
    let rows = match find
        .order_by_asc(audit_log::Column::At)
        .order_by_asc(audit_log::Column::Id)
        .limit(limit)
        .all(&state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => return error_to_response(crate::Error::Database(e)).into_response(),
    };
    let server = crate::config::env::text("HOSTNAME");
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({ "resource": crate::audit_export::to_audit_event(r, server.as_deref()) })
        })
        .collect();
    let ctx = crate::api::rest::handlers::request_audit_ctx(&headers);
    if let Err(e) = AuditLogRepository::log_with(
        &state.db,
        "audit_export",
        Uuid::new_v4(),
        "export_balp",
        None,
        None,
        None,
        Some(serde_json::json!({
            "from": from, "to": to, "entity_type": q.entity_type, "entity_id": q.entity_id,
            "count": entries.len(),
        })),
        &ctx,
    )
    .await
    {
        return error_to_response(e).into_response();
    }
    let body = serde_json::json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": entries.len(),
        "entry": entries,
    });
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/fhir+json; fhirVersion=4.0",
        )],
        body.to_string(),
    )
        .into_response()
}

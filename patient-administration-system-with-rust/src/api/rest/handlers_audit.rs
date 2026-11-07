//! REST handlers for audit query.

use crate::db::sql::Conn;
use axa::extract::{Path, Query};
use axum::extract::State;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page};
use crate::db::repositories::audit::AuditLogRepository;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
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
    params(LimitQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::db::entities::audit_log::Model>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
    params(EntityQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::db::entities::audit_log::Model>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
    params(("id" = String, Path, description = "Path parameter `id`"), LimitQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::db::entities::audit_log::Model>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
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
    params(BalpQuery),
    responses(
        (status = 200, description = "A FHIR R4 Bundle of AuditEvent (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_balp(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<BalpQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

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
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let mut sql = "SELECT * FROM audit_log WHERE at >= $1 AND at < $2".to_string();
    let mut params: Vec<crate::db::sql::Param> = vec![from.into(), to.into()];
    if let Some(t) = &q.entity_type {
        params.push(t.as_str().into());
        sql.push_str(&format!(" AND entity_type = ${}", params.len()));
    }
    if let Some(id) = q.entity_id {
        params.push(id.into());
        sql.push_str(&format!(" AND entity_id = ${}", params.len()));
    }
    params.push(limit.into());
    sql.push_str(&format!(" ORDER BY at, id LIMIT ${}", params.len()));
    let rows = match state
        .db
        .all_as::<audit_log::Model>(crate::db::sql::Stmt::new(sql, params))
        .await
    {
        Ok(r) => r,
        Err(e) => return error_to_response(e).into_response(),
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

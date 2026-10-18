//! REST handlers for Coverage CRUD.
//!
//! Takes the full `Coverage` shape on POST/PUT (caller mints id +
//! timestamps client-side if they want; otherwise the server fills both).

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
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::coverage::{Coverage, CoverageStatus};

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

/// `POST /api/coverages` — create a coverage. Body is the full Coverage
/// JSON (client may omit id + timestamps; server fills them).
pub async fn create_coverage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Coverage>,
) -> HandlerResult {
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match CoverageRepository::create(&state.db, &body).await {
        Ok(c) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "coverage",
                c.id,
                "create",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await;
            let _ = OutboxRepository::publish(
                &state.db,
                "CoverageCreated",
                &serde_json::json!({ "coverage_id": c.id, "person_id": c.person_id }),
            )
            .await;
            ok(serde_json::to_value(c).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/coverages/:id`.
pub async fn get_coverage(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => ok(serde_json::to_value(c).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("coverage {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/coverages/:id`.
pub async fn update_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(mut body): Json<Coverage>,
) -> HandlerResult {
    body.id = id;
    body.updated_at = chrono::Utc::now();
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match CoverageRepository::update(&state.db, &body).await {
        Ok(c) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "coverage",
                c.id,
                "update",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await;
            ok(serde_json::to_value(c).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/coverages/:id` — soft-cancel: flips status to Cancelled.
pub async fn delete_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match CoverageRepository::set_status(&state.db, id, CoverageStatus::Cancelled).await {
        Ok(c) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "coverage",
                id,
                "cancel",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await;
            let _ = OutboxRepository::publish(
                &state.db,
                "CoverageCancelled",
                &serde_json::json!({ "coverage_id": id }),
            )
            .await;
            ok(serde_json::to_value(c).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/:id/coverages` — every coverage for the person.
pub async fn list_coverages_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_patient(&state.db, person_id).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/accounts/:id/coverages` — coverages linked to a billing account.
pub async fn list_coverages_for_account(
    State(state): State<AppState>,
    Path(account_id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_account(&state.db, account_id).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

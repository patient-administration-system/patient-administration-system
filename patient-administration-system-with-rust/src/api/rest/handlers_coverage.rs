//! REST handlers for Coverage CRUD.
//!
//! Takes the full `Coverage` shape on POST/PUT (caller mints id +
//! timestamps client-side if they want; otherwise the server fills both).

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::error::LogErr;
use crate::models::coverage::{Coverage, CoverageStatus};

/// `POST /api/coverages` — create a coverage. Body is the full Coverage
/// JSON (client may omit id + timestamps; server fills them).
#[utoipa::path(
    post,
    path = "/api/coverages",
    tag = "Coverages",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
            AuditLogRepository::log(
                &state.db,
                "coverage",
                c.id,
                "create",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await
            .log_error("audit log write");
            OutboxRepository::publish(
                &state.db,
                "CoverageCreated",
                &serde_json::json!({ "coverage_id": c.id, "person_id": c.person_id }),
            )
            .await
            .log_error("outbox event");
            ok_json(c)
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/coverages/:id`.
#[utoipa::path(
    get,
    path = "/api/coverages/{id}",
    tag = "Coverages",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_coverage(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => ok_json(c),
        Ok(None) => error_to_response(Error::not_found(format!("coverage {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/coverages/:id`.
#[utoipa::path(
    put,
    path = "/api/coverages/{id}",
    tag = "Coverages",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
            AuditLogRepository::log(
                &state.db,
                "coverage",
                c.id,
                "update",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await
            .log_error("audit log write");
            ok_json(c)
        }
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/coverages/:id` — soft-cancel: flips status to Cancelled.
#[utoipa::path(
    delete,
    path = "/api/coverages/{id}",
    tag = "Coverages",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn delete_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match CoverageRepository::set_status(&state.db, id, CoverageStatus::Cancelled).await {
        Ok(c) => {
            AuditLogRepository::log(
                &state.db,
                "coverage",
                id,
                "cancel",
                None,
                Some(serde_json::to_value(&c).unwrap_or_default()),
                &ctx,
            )
            .await
            .log_error("audit log write");
            OutboxRepository::publish(
                &state.db,
                "CoverageCancelled",
                &serde_json::json!({ "coverage_id": id }),
            )
            .await
            .log_error("outbox event");
            ok_json(c)
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/:id/coverages` — every coverage for the person.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/coverages",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_coverages_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_patient(&state.db, person_id).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/accounts/:id/coverages` — coverages linked to a billing account.
#[utoipa::path(
    get,
    path = "/api/accounts/{id}/coverages",
    tag = "Accounts",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_coverages_for_account(
    State(state): State<AppState>,
    Path(account_id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_account(&state.db, account_id).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

//! REST handlers for Coverage CRUD.
//!
//! Takes the full `Coverage` shape on POST/PUT (caller mints id +
//! timestamps client-side if they want; otherwise the server fills both).

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use sea_orm::TransactionTrait;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::coverage::{Coverage, CoverageStatus};

/// `POST /api/coverages` — create a coverage. Body is the full Coverage
/// JSON (client may omit id + timestamps; server fills them).
#[utoipa::path(
    post,
    path = "/api/coverages",
    tag = "Coverages",
    request_body = Coverage,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Coverage>),
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let c = CoverageRepository::create(&txn, &body).await?;
        AuditLogRepository::log(
            &txn,
            "coverage",
            c.id,
            "create",
            None,
            Some(serde_json::to_value(&c).unwrap_or_default()),
            &ctx,
        )
        .await?;
        OutboxRepository::publish(
            &txn,
            "CoverageCreated",
            &serde_json::json!({ "coverage_id": c.id, "person_id": c.person_id }),
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(c)
    };
    match run.await {
        Ok(c) => ok_json(c),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Coverage>),
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
    request_body = Coverage,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Coverage>),
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let c = CoverageRepository::update(&txn, &body).await?;
        AuditLogRepository::log(
            &txn,
            "coverage",
            c.id,
            "update",
            None,
            Some(serde_json::to_value(&c).unwrap_or_default()),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(c)
    };
    match run.await {
        Ok(c) => ok_json(c),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Coverage>),
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let c = CoverageRepository::set_status(&txn, id, CoverageStatus::Cancelled).await?;
        AuditLogRepository::log(
            &txn,
            "coverage",
            id,
            "cancel",
            None,
            Some(serde_json::to_value(&c).unwrap_or_default()),
            &ctx,
        )
        .await?;
        OutboxRepository::publish(
            &txn,
            "CoverageCancelled",
            &serde_json::json!({ "coverage_id": id }),
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(c)
    };
    match run.await {
        Ok(c) => ok_json(c),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::coverage::Coverage>>),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<Coverage>>),
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

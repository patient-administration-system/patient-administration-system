//! REST handlers for operational admin: outbox diagnostics + DLQ.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use sea_orm::TransactionTrait;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::outbox::{DeadLetterRepository, OutboxRepository};

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_limit() -> u64 {
    50
}

const MAX_LIMIT: u64 = 500;

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

/// `GET /api/admin/outbox/unpublished?limit=N`.
#[utoipa::path(
    get,
    path = "/api/admin/outbox/unpublished",
    tag = "Admin",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_unpublished_outbox(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = q.limit.min(MAX_LIMIT);
    match OutboxRepository::fetch_unpublished(&state.db, limit).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/admin/outbox/dead-letters?limit=N`.
#[utoipa::path(
    get,
    path = "/api/admin/outbox/dead-letters",
    tag = "Admin",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_dead_letters(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = q.limit.min(MAX_LIMIT);
    match DeadLetterRepository::list(&state.db, limit).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/admin/outbox/dead-letters/:id/replay` — re-inserts the
/// dead-letter row into `outbox_events` (retry_count=0) and deletes the
/// DLQ row, atomically. Returns the new outbox row id.
#[utoipa::path(
    post,
    path = "/api/admin/outbox/dead-letters/{id}/replay",
    tag = "Admin",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn replay_dead_letter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let dl_id = id;
    let result = state
        .db
        .transaction::<_, Uuid, Error>(|txn| {
            let ctx = ctx.clone();
            Box::pin(async move {
                let new_id = DeadLetterRepository::replay(txn, dl_id).await?;
                AuditLogRepository::log(
                    txn,
                    "outbox_dead_letter",
                    dl_id,
                    "replay",
                    None,
                    Some(serde_json::json!({ "new_outbox_id": new_id })),
                    &ctx,
                )
                .await?;
                Ok(new_id)
            })
        })
        .await;
    match result {
        Ok(new_id) => ok(serde_json::json!({
            "dead_letter_id": dl_id,
            "new_outbox_id": new_id,
        })),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

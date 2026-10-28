//! REST handlers for operational admin: outbox diagnostics + DLQ.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
};
use sea_orm::TransactionTrait;
use serde::Deserialize;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_page};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::outbox::{DeadLetterRepository, OutboxRepository};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

/// What replaying a dead letter made.
#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Replayed {
    pub dead_letter_id: Uuid,
    pub new_outbox_id: Uuid,
}

/// `GET /api/admin/outbox/unpublished?limit=N`.
#[utoipa::path(
    get,
    path = "/api/admin/outbox/unpublished",
    tag = "Admin",
    params(LimitQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::db::entities::outbox_event::Model>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_unpublished_outbox(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let fetch = u64::try_from(crate::paging::ask_for(limit)).unwrap_or(0);
    match OutboxRepository::fetch_unpublished(&state.db, fetch).await {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/admin/outbox/dead-letters?limit=N`.
#[utoipa::path(
    get,
    path = "/api/admin/outbox/dead-letters",
    tag = "Admin",
    params(LimitQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::db::entities::outbox_dead_letter::Model>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_dead_letters(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 50) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    let fetch = u64::try_from(crate::paging::ask_for(limit)).unwrap_or(0);
    match DeadLetterRepository::list(&state.db, fetch).await {
        Ok(rows) => ok_page(rows, limit),
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
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Replayed>),
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
        Ok(new_id) => ok_json(Replayed {
            dead_letter_id: dl_id,
            new_outbox_id: new_id,
        }),
        Err(e) => error_to_response(Error::from(e)),
    }
}

/// `GET /api/admin/metrics`: counters and gauges in the Prometheus text format (QUAL-16): requests by status
/// class, background job runs and when each last finished, outbox events waiting and dead-lettered, and open
/// staff call tasks. No identifiers. Administrators only.
#[utoipa::path(
    get,
    path = "/api/admin/metrics",
    tag = "Admin",
    responses(
        (status = 200, description = "Prometheus text format (`text/plain`)"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn metrics(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match crate::metrics::render(&state.db).await {
        Ok(text) => (
            axum::http::StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4",
            )],
            text,
        )
            .into_response(),
        Err(e) => error_to_response(e).into_response(),
    }
}

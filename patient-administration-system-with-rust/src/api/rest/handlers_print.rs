//! REST handlers for the postal letter print queue (NOT-7).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json, request_audit_ctx};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct QueueQuery {
    pub limit: Option<i64>,
}

/// `GET /api/print-queue`: letters waiting to be posted, oldest first, with the total waiting.
#[utoipa::path(
    get,
    path = "/api/print-queue",
    tag = "Print queue",
    params(QueueQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<PrintQueue>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn queue(State(state): State<AppState>, Query(q): Query<QueueQuery>) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state.printing.queue(limit).await {
        Ok((total, items)) => ok_json(PrintQueue {
            waiting: total,
            letters: items,
        }),
        Err(e) => error_to_response(e),
    }
}

/// The letters waiting to be posted.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PrintQueue {
    pub waiting: i64,
    pub letters: Vec<crate::printing::QueuedLetter>,
}

/// How many letters a batch flipped to sent.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BatchSent {
    pub letters_sent: usize,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ExportBody {
    pub created_by: String,
    #[serde(default)]
    pub limit: Option<i64>,
    /// `csv` for the mail house's file; JSON otherwise.
    #[serde(default)]
    pub format: Option<String>,
}

/// `POST /api/print-batches`: make a batch of the oldest queued letters that have an address, and return the
/// export (JSON, or CSV with `"format": "csv"`). The letters stay `pending` until the batch is marked sent.
#[utoipa::path(
    post,
    path = "/api/print-batches",
    tag = "Print batches",
    request_body = ExportBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::printing::Batch>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<ExportBody>,
) -> Response {
    match state
        .printing
        .export_batch(
            b.limit.unwrap_or(crate::printing::MAX_BATCH),
            &b.created_by,
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(batch) if b.format.as_deref() == Some("csv") => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                (
                    header::HeaderName::from_static("x-batch-id"),
                    batch.batch_id.to_string(),
                ),
            ],
            batch.to_csv(),
        )
            .into_response(),
        Ok(batch) => ok_json(batch).into_response(),
        Err(e) => error_to_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SentBody {
    pub sent_by: String,
}

/// `POST /api/print-batches/{id}/sent`: the mail house has the batch. Flips exactly its letters to sent.
#[utoipa::path(
    post,
    path = "/api/print-batches/{id}/sent",
    tag = "Print batches",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = SentBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<BatchSent>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mark_sent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<SentBody>,
) -> HandlerResult {
    match state
        .printing
        .mark_sent(id, &b.sent_by, &request_audit_ctx(&headers))
        .await
    {
        Ok(n) => ok_json(BatchSent { letters_sent: n }),
        Err(e) => error_to_response(e),
    }
}

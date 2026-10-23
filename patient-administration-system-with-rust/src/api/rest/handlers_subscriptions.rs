//! REST handlers for event subscriptions (NEW-4). Administrator only (`user_auth::administrator_only`).

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok, request_audit_ctx};
use crate::subscriptions::SubscriptionService;
use crate::{Error, Result};

/// Serialize or answer with an internal error, never a `null` (QUAL-27).
fn json<T: Serialize>(value: Result<T>) -> HandlerResult {
    match value.and_then(|v| {
        serde_json::to_value(v)
            .map_err(|e| Error::internal(format!("response could not be serialized: {e}")))
    }) {
        Ok(v) => ok(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    /// `appointment` or `waiting-list`.
    pub topic: String,
    /// Where notifications are posted: `https`, or `http` on localhost.
    pub endpoint: String,
    /// The name of the environment variable holding the signing secret, like `PAS_SUB_SECRET_PARTNER`.
    pub secret_env: Option<String>,
    /// The administrator, when the request does not carry an identified caller.
    pub created_by: Option<String>,
}

/// `POST /api/admin/subscriptions`: register a subscriber.
#[utoipa::path(
    post,
    path = "/api/admin/subscriptions",
    tag = "Admin",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateBody>,
) -> HandlerResult {
    let ctx = request_audit_ctx(&headers);
    let by = ctx
        .user_id
        .clone()
        .or(b.created_by.clone())
        .unwrap_or_default();
    json(
        SubscriptionService::new(state.db.clone())
            .create(&b.topic, &b.endpoint, b.secret_env.as_deref(), &by, &ctx)
            .await,
    )
}

/// `GET /api/admin/subscriptions`: every subscription with its counts of pending, delivered and failed.
#[utoipa::path(
    get,
    path = "/api/admin/subscriptions",
    tag = "Admin",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(State(state): State<AppState>) -> HandlerResult {
    json(SubscriptionService::new(state.db.clone()).list().await)
}

/// `DELETE /api/admin/subscriptions/{id}`: switch a subscription off; undelivered notifications are dropped.
#[utoipa::path(
    delete,
    path = "/api/admin/subscriptions/{id}",
    tag = "Admin",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn end(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    json(
        SubscriptionService::new(state.db.clone())
            .end(id, &request_audit_ctx(&headers))
            .await,
    )
}

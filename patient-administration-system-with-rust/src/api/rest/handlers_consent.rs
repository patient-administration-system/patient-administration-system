//! REST handlers for Consent.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::consent::ConsentRepository;
use crate::models::consent::Consent;

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

/// `POST /api/patients/:id/consents` — record a consent.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/consents",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create_consent(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
    headers: HeaderMap,
    Json(mut body): Json<Consent>,
) -> HandlerResult {
    body.person_id = person_id;
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match ConsentRepository::create(&state.db, &body).await {
        Ok(c) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "consent",
                c.id,
                "create",
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

/// `GET /api/patients/:id/consents` — list consents for the person.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/consents",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_consents_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match ConsentRepository::list_for_patient(&state.db, person_id).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/consents/:id/revoke`.
#[utoipa::path(
    post,
    path = "/api/consents/{id}/revoke",
    tag = "Consents",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn revoke_consent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match ConsentRepository::revoke(&state.db, id).await {
        Ok(c) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "consent",
                id,
                "revoke",
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

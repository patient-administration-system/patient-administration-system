//! REST handlers for Consent.

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
use crate::db::repositories::consent::ConsentRepository;
use crate::models::consent::Consent;

/// `POST /api/patients/:id/consents` — record a consent.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/consents",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = Consent,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::consent::Consent>),
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let c = ConsentRepository::create(&txn, &body).await?;
        AuditLogRepository::log(
            &txn,
            "consent",
            c.id,
            "create",
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

/// `GET /api/patients/:id/consents` — list consents for the person.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/consents",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::consent::Consent>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_consents_for_person(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match ConsentRepository::list_for_patient(&state.db, person_id).await {
        Ok(rows) => ok_json(rows),
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let c = ConsentRepository::revoke(&txn, id).await?;
        AuditLogRepository::log(
            &txn,
            "consent",
            id,
            "revoke",
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

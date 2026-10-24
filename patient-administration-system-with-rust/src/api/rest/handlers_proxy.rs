//! REST handlers for carer and proxy authority (NEW-7).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok, ok_json, ok_page, request_audit_ctx,
};
use crate::models::appointment::{Appointment, CancelledBy};
use crate::proxy::{NewAuthority, ProxyService, proxy_context};
use crate::scheduling::CancelDetails;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GrantBody {
    pub proxy_person_id: Uuid,
    /// How the proxy relates to the patient (a carer, a daughter, an attorney).
    pub relationship: String,
    /// `patient_consent`, `parental_responsibility`, `lasting_power_of_attorney`, `court_appointed` or `other`.
    pub basis: String,
    /// A non-empty subset of `receive`, `confirm`, `cancel`, `rebook`.
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub recorded_by: String,
}

/// `POST /api/patients/{id}/proxies`: a named person records who may act for the patient, how far, on what
/// basis and until when. Audited.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/proxies",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = GrantBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn grant(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<GrantBody>,
) -> HandlerResult {
    match ProxyService::new(state.db.clone())
        .grant(
            id,
            &NewAuthority {
                proxy_person_id: b.proxy_person_id,
                relationship: &b.relationship,
                basis: &b.basis,
                scopes: &b.scopes,
                expires_at: b.expires_at,
                recorded_by: &b.recorded_by,
            },
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Include expired and revoked authorities (default false).
    pub include_inactive: Option<bool>,
    pub limit: Option<i64>,
}

/// `GET /api/patients/{id}/proxies`: who may act for the patient. Paged (NFR-23).
#[utoipa::path(
    get,
    path = "/api/patients/{id}/proxies",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match ProxyService::new(state.db.clone())
        .for_patient(
            id,
            q.include_inactive.unwrap_or(false),
            state.clock.now(),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokeBody {
    pub revoked_by: String,
}

/// `POST /api/proxies/{id}/revoke`: a named person withdraws an authority. Once; audited.
#[utoipa::path(
    post,
    path = "/api/proxies/{id}/revoke",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = RevokeBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<RevokeBody>,
) -> HandlerResult {
    match ProxyService::new(state.db.clone())
        .revoke(
            id,
            &b.revoked_by,
            state.clock.now(),
            &request_audit_ctx(&headers),
        )
        .await
    {
        Ok(()) => ok(serde_json::json!({ "id": id, "status": "revoked" })),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ActingQuery {
    /// The authenticated proxy, as the portal knows them.
    pub proxy_person_id: Uuid,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ActingBody {
    pub proxy_person_id: Uuid,
    /// For a rebook: the slot to move to.
    pub slot_id: Option<Uuid>,
}

/// The appointment's patient, or `NotFound`.
async fn appointment(state: &AppState, id: Uuid) -> crate::Result<Appointment> {
    crate::db::repositories::appointment::AppointmentRepository::find_by_id(&state.db, id)
        .await?
        .ok_or_else(|| crate::Error::not_found(format!("appointment {id}")))
}

/// `GET /api/proxy/patients/{id}/appointments?proxy_person_id=`: the patient's appointments, for a proxy
/// with the `receive` scope.
#[utoipa::path(
    get,
    path = "/api/proxy/patients/{id}/appointments",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn appointments(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ActingQuery>,
) -> HandlerResult {
    if let Err(e) = ProxyService::new(state.db.clone())
        .authorise(id, q.proxy_person_id, "receive", state.clock.now())
        .await
    {
        return error_to_response(e);
    }
    match crate::db::repositories::appointment::AppointmentRepository::list_by_patient(
        &state.db, id,
    )
    .await
    {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/proxy/appointments/{id}/cancel`: a proxy with the `cancel` scope cancels the patient's
/// appointment. Audited as the proxy acting for the patient.
#[utoipa::path(
    post,
    path = "/api/proxy/appointments/{id}/cancel",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ActingBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ActingBody>,
) -> HandlerResult {
    let run = async {
        let appt = appointment(&state, id).await?;
        let authority = ProxyService::new(state.db.clone())
            .authorise(
                appt.person_id,
                b.proxy_person_id,
                "cancel",
                state.clock.now(),
            )
            .await?;
        let ctx = proxy_context(&request_audit_ctx(&headers), &authority);
        state
            .scheduling
            .cancel_with(
                id,
                CancelDetails {
                    cancelled_by: CancelledBy::Patient,
                    barrier_code: None,
                    legacy_reason: None,
                },
                &ctx,
            )
            .await
    };
    match run.await {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/proxy/appointments/{id}/rebook`: a proxy with the `rebook` scope moves the patient's
/// appointment to another slot (`slot_id`). Audited as the proxy acting for the patient.
#[utoipa::path(
    post,
    path = "/api/proxy/appointments/{id}/rebook",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ActingBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rebook(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(b): Json<ActingBody>,
) -> HandlerResult {
    let run = async {
        let slot = b
            .slot_id
            .ok_or_else(|| crate::Error::validation("slot_id is required to rebook"))?;
        let appt = appointment(&state, id).await?;
        let authority = ProxyService::new(state.db.clone())
            .authorise(
                appt.person_id,
                b.proxy_person_id,
                "rebook",
                state.clock.now(),
            )
            .await?;
        let ctx = proxy_context(&request_audit_ctx(&headers), &authority);
        let (_, new) = state.scheduling.rebook(id, slot, &ctx).await?;
        Ok::<_, crate::Error>(new)
    };
    match run.await {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

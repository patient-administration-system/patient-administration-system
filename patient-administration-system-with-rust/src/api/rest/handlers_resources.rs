//! REST handlers for the resources (ward / bed) surface.
//!
//! Phase 3b1 restores the read + status-change endpoints that the
//! existing [`crate::resources::ResourcesService`] already provides
//! plus a bed listing helper. Facility / ward / room creation is
//! intentionally deferred to Phase 3b2 — it needs FacilityRepository
//! scaffolding that wasn't in the v0.43.0 split (creation lived
//! inline in handlers.rs and was deleted in Phase 2).

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::facility::{FacilityRepository, RoomRepository, WardRepository};
use crate::models::facility::{Bed, BedStatus, Facility, Room, Ward};

#[derive(Debug, Deserialize)]
pub struct SetBedStatusBody {
    pub status: BedStatus,
}

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok_value(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `GET /api/wards/:id/occupancy` — counts of beds by status.
pub async fn ward_occupancy(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match state.resources.ward_occupancy(ward_id).await {
        Ok(occ) => ok_value(serde_json::to_value(occ).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/beds/:id/status` — state-machine-checked bed status change.
/// Body: `{ "status": "Available" }`.
pub async fn set_bed_status(
    State(state): State<AppState>,
    Path(bed_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SetBedStatusBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .resources
        .set_bed_status(bed_id, body.status, &ctx)
        .await
    {
        Ok(bed) => ok_value(serde_json::to_value(bed).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id/beds` — list beds in the ward.
pub async fn list_beds_by_ward(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match BedRepository::list_by_ward(&state.db, ward_id).await {
        Ok(beds) => ok_value(serde_json::to_value(beds).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/beds/:id` — fetch a bed by id.
pub async fn get_bed(State(state): State<AppState>, Path(bed_id): Path<Uuid>) -> HandlerResult {
    match BedRepository::find_by_id(&state.db, bed_id).await {
        Ok(Some(bed)) => ok_value(serde_json::to_value(bed).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("bed {bed_id}"))),
        Err(e) => error_to_response(e),
    }
}

// ─── Facility / Ward / Room / Bed CRUD ─────────────────────────────────

async fn write_audit_create(
    db: &sea_orm::DatabaseConnection,
    entity_type: &str,
    id: Uuid,
    payload: Value,
    ctx: &RepoUserContext,
) {
    let _ = AuditLogRepository::log(db, entity_type, id, "create", None, Some(payload), ctx).await;
}

/// `POST /api/facilities` — create a facility from a `Facility` JSON body.
pub async fn create_facility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Facility>,
) -> HandlerResult {
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match FacilityRepository::create(&state.db, &body).await {
        Ok(f) => {
            write_audit_create(
                &state.db,
                "facility",
                f.id,
                serde_json::to_value(&f).unwrap_or_default(),
                &ctx,
            )
            .await;
            ok_value(serde_json::to_value(f).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities/:id`.
pub async fn get_facility(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match FacilityRepository::find_by_id(&state.db, id).await {
        Ok(Some(f)) => ok_value(serde_json::to_value(f).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("facility {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities` — list active facilities.
pub async fn list_facilities(State(state): State<AppState>) -> HandlerResult {
    match FacilityRepository::list(&state.db).await {
        Ok(rows) => ok_value(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/wards`.
pub async fn create_ward(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Ward>,
) -> HandlerResult {
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match WardRepository::create(&state.db, &body).await {
        Ok(w) => {
            write_audit_create(
                &state.db,
                "ward",
                w.id,
                serde_json::to_value(&w).unwrap_or_default(),
                &ctx,
            )
            .await;
            ok_value(serde_json::to_value(w).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id`.
pub async fn get_ward(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match WardRepository::find_by_id(&state.db, id).await {
        Ok(Some(w)) => ok_value(serde_json::to_value(w).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("ward {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities/:id/wards`.
pub async fn list_wards_by_facility(
    State(state): State<AppState>,
    Path(facility_id): Path<Uuid>,
) -> HandlerResult {
    match WardRepository::list_by_facility(&state.db, facility_id).await {
        Ok(rows) => ok_value(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rooms`.
pub async fn create_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Room>,
) -> HandlerResult {
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match RoomRepository::create(&state.db, &body).await {
        Ok(r) => {
            write_audit_create(
                &state.db,
                "room",
                r.id,
                serde_json::to_value(&r).unwrap_or_default(),
                &ctx,
            )
            .await;
            ok_value(serde_json::to_value(r).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/rooms/:id`.
pub async fn get_room(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match RoomRepository::find_by_id(&state.db, id).await {
        Ok(Some(r)) => ok_value(serde_json::to_value(r).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("room {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id/rooms`.
pub async fn list_rooms_by_ward(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match RoomRepository::list_by_ward(&state.db, ward_id).await {
        Ok(rows) => ok_value(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/beds`.
pub async fn create_bed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Bed>,
) -> HandlerResult {
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match BedRepository::create(&state.db, &body).await {
        Ok(b) => {
            write_audit_create(
                &state.db,
                "bed",
                b.id,
                serde_json::to_value(&b).unwrap_or_default(),
                &ctx,
            )
            .await;
            ok_value(serde_json::to_value(b).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

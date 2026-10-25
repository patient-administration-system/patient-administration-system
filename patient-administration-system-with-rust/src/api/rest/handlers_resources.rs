//! REST handlers for the resources surface: ward and bed reads, bed status changes and a bed listing on top
//! of [`crate::resources::ResourcesService`], and facility, ward and room creation and reads on top of the
//! facility repositories.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use sea_orm::TransactionTrait;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::facility::{FacilityRepository, RoomRepository, WardRepository};
use crate::models::facility::{Bed, BedStatus, Facility, Room, Ward};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetBedStatusBody {
    pub status: BedStatus,
}

/// `GET /api/wards/:id/occupancy` — counts of beds by status.
#[utoipa::path(
    get,
    path = "/api/wards/{id}/occupancy",
    tag = "Wards",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::resources::WardOccupancy>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn ward_occupancy(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match state.resources.ward_occupancy(ward_id).await {
        Ok(occ) => ok_json(occ),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/beds/:id/status` — state-machine-checked bed status change.
/// Body: `{ "status": "Available" }`.
#[utoipa::path(
    put,
    path = "/api/beds/{id}/status",
    tag = "Beds",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = SetBedStatusBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Bed>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
        Ok(bed) => ok_json(bed),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id/beds` — list beds in the ward.
#[utoipa::path(
    get,
    path = "/api/wards/{id}/beds",
    tag = "Wards",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::facility::Bed>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_beds_by_ward(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match BedRepository::list_by_ward(&state.db, ward_id).await {
        Ok(beds) => ok_json(beds),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/beds/:id` — fetch a bed by id.
#[utoipa::path(
    get,
    path = "/api/beds/{id}",
    tag = "Beds",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Bed>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_bed(State(state): State<AppState>, Path(bed_id): Path<Uuid>) -> HandlerResult {
    match BedRepository::find_by_id(&state.db, bed_id).await {
        Ok(Some(bed)) => ok_json(bed),
        Ok(None) => error_to_response(Error::not_found(format!("bed {bed_id}"))),
        Err(e) => error_to_response(e),
    }
}

// ─── Facility / Ward / Room / Bed CRUD ─────────────────────────────────

/// The audit row of a create. It is written in the transaction of the create, so a failure here fails the
/// request and keeps nothing (NFR-25).
async fn write_audit_create<C: sea_orm::ConnectionTrait>(
    db: &C,
    entity_type: &str,
    id: Uuid,
    payload: Value,
    ctx: &RepoUserContext,
) -> crate::Result<()> {
    AuditLogRepository::log(db, entity_type, id, "create", None, Some(payload), ctx).await
}

/// `POST /api/facilities` — create a facility from a `Facility` JSON body.
#[utoipa::path(
    post,
    path = "/api/facilities",
    tag = "Facilities",
    request_body = Facility,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Facility>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let f = FacilityRepository::create(&txn, &body).await?;
        write_audit_create(
            &txn,
            "facility",
            f.id,
            serde_json::to_value(&f).unwrap_or_default(),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(f)
    };
    match run.await {
        Ok(f) => ok_json(f),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities/:id`.
#[utoipa::path(
    get,
    path = "/api/facilities/{id}",
    tag = "Facilities",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Facility>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_facility(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match FacilityRepository::find_by_id(&state.db, id).await {
        Ok(Some(f)) => ok_json(f),
        Ok(None) => error_to_response(Error::not_found(format!("facility {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities` — list active facilities.
#[utoipa::path(
    get,
    path = "/api/facilities",
    tag = "Facilities",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::facility::Facility>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_facilities(State(state): State<AppState>) -> HandlerResult {
    match FacilityRepository::list(&state.db).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/wards`.
#[utoipa::path(
    post,
    path = "/api/wards",
    tag = "Wards",
    request_body = Ward,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Ward>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let w = WardRepository::create(&txn, &body).await?;
        write_audit_create(
            &txn,
            "ward",
            w.id,
            serde_json::to_value(&w).unwrap_or_default(),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(w)
    };
    match run.await {
        Ok(w) => ok_json(w),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id`.
#[utoipa::path(
    get,
    path = "/api/wards/{id}",
    tag = "Wards",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Ward>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_ward(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match WardRepository::find_by_id(&state.db, id).await {
        Ok(Some(w)) => ok_json(w),
        Ok(None) => error_to_response(Error::not_found(format!("ward {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facilities/:id/wards`.
#[utoipa::path(
    get,
    path = "/api/facilities/{id}/wards",
    tag = "Facilities",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::facility::Ward>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_wards_by_facility(
    State(state): State<AppState>,
    Path(facility_id): Path<Uuid>,
) -> HandlerResult {
    match WardRepository::list_by_facility(&state.db, facility_id).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/rooms`.
#[utoipa::path(
    post,
    path = "/api/rooms",
    tag = "Rooms",
    request_body = Room,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Room>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let r = RoomRepository::create(&txn, &body).await?;
        write_audit_create(
            &txn,
            "room",
            r.id,
            serde_json::to_value(&r).unwrap_or_default(),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(r)
    };
    match run.await {
        Ok(r) => ok_json(r),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/rooms/:id`.
#[utoipa::path(
    get,
    path = "/api/rooms/{id}",
    tag = "Rooms",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Room>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_room(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match RoomRepository::find_by_id(&state.db, id).await {
        Ok(Some(r)) => ok_json(r),
        Ok(None) => error_to_response(Error::not_found(format!("room {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/wards/:id/rooms`.
#[utoipa::path(
    get,
    path = "/api/wards/{id}/rooms",
    tag = "Wards",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::facility::Room>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_rooms_by_ward(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match RoomRepository::list_by_ward(&state.db, ward_id).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/beds`.
#[utoipa::path(
    post,
    path = "/api/beds",
    tag = "Beds",
    request_body = Bed,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::facility::Bed>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let b = BedRepository::create(&txn, &body).await?;
        write_audit_create(
            &txn,
            "bed",
            b.id,
            serde_json::to_value(&b).unwrap_or_default(),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(b)
    };
    match run.await {
        Ok(b) => ok_json(b),
        Err(e) => error_to_response(e),
    }
}

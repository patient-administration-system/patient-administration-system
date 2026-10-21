//! REST handlers for the scheduling surface.
//!
//! Phase 3b1 restores slot booking + the appointment lifecycle on top of
//! the existing [`crate::scheduling::SchedulingService`]. Bulk slot
//! generation, appointment series, and the `mark_no_show` path land in
//! Phase 3b2.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;

#[derive(Debug, Deserialize)]
pub struct BookBody {
    pub person_id: Uuid,
}

#[derive(Debug, Deserialize, Default)]
pub struct CancelBody {
    /// Free text from older callers. Accepted and ignored; use `barrier_code`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Who is cancelling. Defaults to `patient`, which is what this endpoint always recorded.
    #[serde(default)]
    pub cancelled_by: Option<crate::models::appointment::CancelledBy>,
    /// The patient's barrier, when they gave one. An unknown code is refused with 422.
    #[serde(default)]
    pub barrier_code: Option<crate::models::appointment::BarrierCode>,
}

#[derive(Debug, Deserialize)]
pub struct BarrierBody {
    pub barrier_code: crate::models::appointment::BarrierCode,
}

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
}

/// `GET /api/appointments?from=<rfc3339>&to=<rfc3339>` — appointments
/// starting in `[from, to)`. The range is capped at 366 days.
#[utoipa::path(
    get,
    path = "/api/appointments",
    tag = "Appointments",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_appointments(
    State(state): State<AppState>,
    Query(q): Query<RangeQuery>,
) -> HandlerResult {
    if q.to <= q.from || q.to - q.from > chrono::Duration::days(366) {
        return error_to_response(crate::Error::validation(
            "`from` must precede `to` by at most 366 days",
        ));
    }
    match crate::db::repositories::appointment::AppointmentRepository::list_between(
        &state.db, q.from, q.to,
    )
    .await
    {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/{id}/appointments` — every appointment for one patient.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/appointments",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_patient_appointments(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
) -> HandlerResult {
    match crate::db::repositories::appointment::AppointmentRepository::list_by_patient(
        &state.db, person_id,
    )
    .await
    {
        Ok(mut rows) => {
            rows.sort_by_key(|a| a.start_datetime);
            ok_json(rows)
        }
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/slots/:id/book` — book the named slot for the supplied
/// person id. Body: `{ "person_id": "<uuid>" }`.
#[utoipa::path(
    post,
    path = "/api/slots/{id}/book",
    tag = "Slots",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn book_slot(
    State(state): State<AppState>,
    Path(slot_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BookBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .scheduling
        .book_slot(slot_id, body.person_id, &ctx)
        .await
    {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/cancel` — cancel a booked appointment.
/// Body: `{ "reason": "<free text>" }` (optional; defaults to
/// `PatientRequest`).
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/cancel",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    body: Option<Json<CancelBody>>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let details = crate::scheduling::CancelDetails {
        cancelled_by: body
            .cancelled_by
            .unwrap_or(crate::models::appointment::CancelledBy::Patient),
        barrier_code: body.barrier_code,
        legacy_reason: None,
    };
    match state
        .scheduling
        .cancel_with(appointment_id, details, &ctx)
        .await
    {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RoomBody {
    /// Where the patient is now; empty or absent clears it.
    #[serde(default)]
    pub room: Option<String>,
}

/// `PUT /api/appointments/{id}/location` — move a patient who has arrived to a room (CORE-4). Audited.
#[utoipa::path(
    put,
    path = "/api/appointments/{id}/location",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn move_to_room(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RoomBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .scheduling
        .move_to_room(appointment_id, body.room.as_deref(), &ctx)
        .await
    {
        Ok(a) => ok_json(a),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/facility/in-facility` — everyone who has arrived and not been completed, longest waiting first,
/// with the minutes they have been here and where they are (CORE-4).
#[utoipa::path(
    get,
    path = "/api/facility/in-facility",
    tag = "Facility",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn in_facility(State(state): State<AppState>) -> HandlerResult {
    match state.scheduling.in_facility().await {
        Ok(v) => ok(Value::Array(v)),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/appointments/{id}/barrier` — record why a cancelled or missed appointment happened,
/// typically after a welfare call (CAP-1).
#[utoipa::path(
    put,
    path = "/api/appointments/{id}/barrier",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn record_barrier(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BarrierBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .scheduling
        .record_barrier(appointment_id, body.barrier_code, &ctx)
        .await
    {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/check-in` — `Booked → Arrived`.
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/check-in",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn check_in_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.check_in(appointment_id, &ctx).await {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/complete` — `Arrived → Fulfilled`.
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/complete",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn complete_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.complete(appointment_id, &ctx).await {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointments/:id/no-show` — `Booked → NoShow`.
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/no-show",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mark_no_show(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.scheduling.mark_no_show(appointment_id, &ctx).await {
        Ok(appt) => ok_json(appt),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/appointments/{id}/rebook-options` — two or three real alternative slots for this
/// appointment (CAP-5). Asking again while the offer stands returns the same offer. `options` is empty
/// when there is nothing to offer; hand the patient to a person then.
#[utoipa::path(
    get,
    path = "/api/appointments/{id}/rebook-options",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rebook_options(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
) -> HandlerResult {
    match state.rebooking.offer(appointment_id).await {
        Ok(o) => ok_json(o),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RebookBody {
    pub offer_id: Uuid,
    pub slot_id: Uuid,
}

/// `POST /api/appointments/{id}/rebook` — take one of the offered times. A slot that was not in the
/// offer is refused (400), a used offer is 409. The old appointment is released as `rescheduled` and the
/// new one booked in one transaction.
#[utoipa::path(
    post,
    path = "/api/appointments/{id}/rebook",
    tag = "Appointments",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn rebook_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RebookBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .rebooking
        .choose(appointment_id, body.offer_id, body.slot_id, &ctx)
        .await
    {
        Ok((old, new)) => ok(serde_json::json!({ "old": old, "new": new })),
        Err(e) => error_to_response(e),
    }
}

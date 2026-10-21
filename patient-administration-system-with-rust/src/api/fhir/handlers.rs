//! FHIR R5 HTTP handlers.
//!
//! `/fhir/Patient/*` and `/fhir/Practitioner/*` proxy `person-service` / `worker-service` and forward their
//! shape verbatim. Encounter, Appointment, Coverage, Schedule, Slot and Location are read from the PAS-owned
//! tables and wrapped as FHIR; Schedule and Slot can also be written. `POST /fhir/Encounter` and
//! `POST /fhir/Appointment` are intentionally not exposed (`503` with a `not-supported` OperationOutcome that
//! names the REST endpoint to use). `POST /fhir` accepts a write Bundle.
//!
//! ## Wire shape
//!
//! Patient and Practitioner forward the upstream's Person / Worker shape unchanged; the PAS does not
//! re-translate it. Callers that need FHIR R5 Person semantics should use `person-service` directly; this
//! surface is a compatibility shim.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::upstream::{PersonClient, PersonCreate, WorkerClient, WorkerCreate};

use super::operation_outcome::{self, OperationOutcome};

type FhirResult = Result<Json<Value>, (StatusCode, Json<OperationOutcome>)>;

/// Map any [`Error`] to a status + OperationOutcome.
/// A resource as JSON. One that cannot be serialized is a logged `500`, never a silent `null`.
fn json_of<T: serde::Serialize>(value: T) -> FhirResult {
    serde_json::to_value(value).map(Json).map_err(|e| {
        error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        )))
    })
}

/// A resource wrapped as FHIR (`wrap_fhir`); one that cannot be serialized is a logged `500`.
fn fhir_of<T: serde::Serialize>(kind: &str, value: T) -> FhirResult {
    match serde_json::to_value(value) {
        Ok(v) => Ok(Json(wrap_fhir(kind, v))),
        Err(e) => Err(error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        )))),
    }
}

fn error_to_response(err: Error) -> (StatusCode, Json<OperationOutcome>) {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "not-found"),
        Error::Validation(_) | Error::Fhir(_) => (StatusCode::BAD_REQUEST, "invalid"),
        Error::InvalidStateTransition(_) | Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
        Error::Upstream(_) => (StatusCode::SERVICE_UNAVAILABLE, "transient"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "exception"),
    };
    (
        status,
        Json(operation_outcome::error(code, err.to_string())),
    )
}

fn need_person_client(state: &AppState) -> Result<&dyn PersonClient, Error> {
    state
        .person_client
        .as_deref()
        .ok_or_else(|| Error::upstream("PERSON_SERVICE_URL unset; FHIR Patient proxy disabled"))
}

fn need_worker_client(state: &AppState) -> Result<&dyn WorkerClient, Error> {
    state.worker_client.as_deref().ok_or_else(|| {
        Error::upstream("WORKER_SERVICE_URL unset; FHIR Practitioner proxy disabled")
    })
}

fn body_to_fields(body: Value) -> Result<serde_json::Map<String, Value>, Error> {
    match body {
        Value::Object(m) => Ok(m),
        other => Err(Error::validation(format!(
            "FHIR body must be a JSON object, got {other:?}"
        ))),
    }
}

// ─── /fhir/Patient/* (proxy person-service) ────────────────────────

#[utoipa::path(
    get,
    path = "/fhir/Patient/{id}",
    tag = "FHIR",
    summary = "Get patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> FhirResult {
    let client = need_person_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let person = client.get(id, &ctx).await.map_err(error_to_response)?;
    json_of(person)
}

#[utoipa::path(
    post,
    path = "/fhir/Patient",
    tag = "FHIR",
    summary = "Create patient",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let client = need_person_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let fields = body_to_fields(body).map_err(error_to_response)?;
    let person = client
        .create(PersonCreate { fields }, &ctx)
        .await
        .map_err(error_to_response)?;
    json_of(person)
}

#[utoipa::path(
    put,
    path = "/fhir/Patient/{id}",
    tag = "FHIR",
    summary = "Update patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn update_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let client = need_person_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let fields = body_to_fields(body).map_err(error_to_response)?;
    let person = client
        .update(id, PersonCreate { fields }, &ctx)
        .await
        .map_err(error_to_response)?;
    json_of(person)
}

#[utoipa::path(
    delete,
    path = "/fhir/Patient/{id}",
    tag = "FHIR",
    summary = "Delete patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn delete_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    let client = need_person_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    client.delete(id, &ctx).await.map_err(error_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/fhir/Patient",
    tag = "FHIR",
    summary = "List patients",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn list_patients(State(state): State<AppState>, headers: HeaderMap) -> FhirResult {
    let client = need_person_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let persons = client
        .search("", 20, &ctx)
        .await
        .map_err(error_to_response)?;
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "entry": persons,
    })))
}

// ─── /fhir/Practitioner/* (proxy worker-service) ───────────────────

#[utoipa::path(
    get,
    path = "/fhir/Practitioner/{id}",
    tag = "FHIR",
    summary = "Get practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> FhirResult {
    let client = need_worker_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let worker = client.get(id, &ctx).await.map_err(error_to_response)?;
    json_of(worker)
}

#[utoipa::path(
    post,
    path = "/fhir/Practitioner",
    tag = "FHIR",
    summary = "Create practitioner",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_practitioner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let client = need_worker_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let fields = body_to_fields(body).map_err(error_to_response)?;
    let worker = client
        .create(WorkerCreate { fields }, &ctx)
        .await
        .map_err(error_to_response)?;
    json_of(worker)
}

#[utoipa::path(
    put,
    path = "/fhir/Practitioner/{id}",
    tag = "FHIR",
    summary = "Update practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn update_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let client = need_worker_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    let fields = body_to_fields(body).map_err(error_to_response)?;
    let worker = client
        .update(id, WorkerCreate { fields }, &ctx)
        .await
        .map_err(error_to_response)?;
    json_of(worker)
}

#[utoipa::path(
    delete,
    path = "/fhir/Practitioner/{id}",
    tag = "FHIR",
    summary = "Delete practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn delete_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    let client = need_worker_client(&state).map_err(error_to_response)?;
    let ctx = user_context_from_headers(&headers);
    client.delete(id, &ctx).await.map_err(error_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ─── PAS-owned FHIR reads ───────────────────────────────────────────────
//
// These return JSON wrapped in a minimal FHIR-shaped envelope. They
// don't translate to typed FhirEncounter / FhirAppointment / FhirCoverage
// shapes. The wire shape is the underlying PAS row JSON with a
// `resourceType` field added so basic FHIR client type-routing works.
// A real translator (and the R4 form Wales uses) is NEW-1 in `tasks.md`.

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::db::repositories::schedule::ScheduleRepository;
use crate::db::repositories::slot::SlotRepository;

fn wrap_fhir(resource_type: &str, mut value: Value) -> Value {
    if let Value::Object(ref mut map) = value {
        map.insert(
            "resourceType".to_string(),
            Value::String(resource_type.to_string()),
        );
    }
    value
}

#[utoipa::path(
    get,
    path = "/fhir/Encounter/{id}",
    tag = "FHIR",
    summary = "Get encounter",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_encounter(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match EncounterRepository::find_by_id(&state.db, id).await {
        Ok(Some(e)) => fhir_of("Encounter", e),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Encounter/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Appointment/{id}",
    tag = "FHIR",
    summary = "Get appointment",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_appointment(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match AppointmentRepository::find_by_id(&state.db, id).await {
        Ok(Some(a)) => fhir_of("Appointment", a),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Appointment/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Coverage/{id}",
    tag = "FHIR",
    summary = "Get coverage",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_coverage(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => fhir_of("Coverage", c),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Coverage/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Schedule/{id}",
    tag = "FHIR",
    summary = "Get schedule",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_schedule(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match ScheduleRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => fhir_of("Schedule", s),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Schedule/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Slot/{id}",
    tag = "FHIR",
    summary = "Get slot",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_slot(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match SlotRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => fhir_of("Slot", s),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Slot/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// FHIR Location read backed by the PAS `Bed` row.
#[utoipa::path(
    get,
    path = "/fhir/Location/{id}",
    tag = "FHIR",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_location(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match BedRepository::find_by_id(&state.db, id).await {
        Ok(Some(b)) => fhir_of("Location", b),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Location/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

// ─── Write surface ──────────────────────────────────────────────────────
//
// FHIR Schedule + Slot create/update/delete persist directly against the
// PAS-owned `schedules` / `slots` tables. Bundle POST validates entries
// for Decision #4 (no Patient or Practitioner entries — those must go
// through person-service / worker-service directly) and dispatches each
// remaining entry by `resourceType`. Encounter + Appointment create are
// left stubbed because both need person-id resolution / state-machine
// orchestration (ADT for Encounter, scheduling service for Appointment)
// — callers should use the typed REST endpoints (`/api/admissions`,
// `/api/appointments`) for those.

use crate::models::schedule::{Schedule, ScheduleOwner, Slot, SlotStatus};

fn upstream_stub(resource: &str) -> (StatusCode, Json<OperationOutcome>) {
    let err = Error::upstream(format!(
        "FHIR {resource} write is intentionally not exposed; use the typed REST \
         endpoint instead (POST /api/admissions for Encounter; \
         POST /api/appointments for Appointment)."
    ));
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(operation_outcome::error("not-supported", err.to_string())),
    )
}

#[utoipa::path(
    post,
    path = "/fhir/Encounter",
    tag = "FHIR",
    summary = "Create encounter",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_encounter() -> FhirResult {
    Err(upstream_stub("Encounter"))
}

#[utoipa::path(
    post,
    path = "/fhir/Appointment",
    tag = "FHIR",
    summary = "Create appointment",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_appointment() -> FhirResult {
    Err(upstream_stub("Appointment"))
}

// ── Schedule writes ──────────────────────────────────────────────────────

/// Read one element of a FHIR-shaped body as its typed R5 datatype, so a malformed one is refused with
/// the reason. Elements, not whole resources: a PUT may carry only the fields it changes.
fn fhir_element<T: serde::de::DeserializeOwned>(value: &Value, what: &str) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|e| Error::validation(format!("{what}: {e}")))
}

fn parse_schedule_owner(body: &Value) -> Result<ScheduleOwner, Error> {
    // FHIR Schedule.actor[0].reference is the canonical owner pointer.
    // We accept either:
    //   { "actor": [{ "reference": "Practitioner/<uuid>" }] }
    //   { "owner": { "kind": "Practitioner", "id": "<uuid>" } }   (PAS native)
    if body.get("actor").is_some() {
        let actor: fhir::r5::types::Reference = body
            .get("actor")
            .and_then(|a| a.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| Error::validation("Schedule.actor[0] is required"))
            .and_then(|v| fhir_element(v, "Schedule.actor[0]"))?;
        let actor_ref = actor
            .reference
            .as_ref()
            .map(|r| r.0.as_str())
            .ok_or_else(|| Error::validation("Schedule.actor[0].reference is required"))?;
        let (kind, id_str) = actor_ref
            .split_once('/')
            .ok_or_else(|| Error::validation(format!("invalid actor reference {actor_ref:?}")))?;
        let id = Uuid::parse_str(id_str)
            .map_err(|e| Error::validation(format!("actor id {id_str:?}: {e}")))?;
        return match kind {
            "Practitioner" => Ok(ScheduleOwner::Practitioner(id)),
            "Location" | "Bed" => Ok(ScheduleOwner::Bed(id)),
            "Room" => Ok(ScheduleOwner::Room(id)),
            "Device" | "Equipment" => Ok(ScheduleOwner::Equipment(id)),
            other => Err(Error::validation(format!(
                "Schedule.actor[0].reference kind {other:?} not supported"
            ))),
        };
    }
    if let Some(owner) = body.get("owner")
        && let (Some(kind), Some(id_str)) = (
            owner.get("kind").and_then(|v| v.as_str()),
            owner.get("id").and_then(|v| v.as_str()),
        )
    {
        let id = Uuid::parse_str(id_str)
            .map_err(|e| Error::validation(format!("owner.id {id_str:?}: {e}")))?;
        return match kind {
            "Practitioner" => Ok(ScheduleOwner::Practitioner(id)),
            "Bed" => Ok(ScheduleOwner::Bed(id)),
            "Room" => Ok(ScheduleOwner::Room(id)),
            other => Err(Error::validation(format!(
                "owner.kind {other:?} not supported"
            ))),
        };
    }
    Err(Error::validation(
        "Schedule requires actor[0].reference or owner { kind, id }",
    ))
}

fn parse_service_type(body: &Value) -> String {
    // FHIR R5 Schedule.serviceType[0].concept.coding[0].code is the
    // canonical place; we also accept a plain "service_type" string for
    // PAS-native callers.
    if let Some(s) = body.get("service_type").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(first) = body
        .get("serviceType")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        && let Ok(service) =
            fhir_element::<fhir::r5::types::CodeableReference>(first, "Schedule.serviceType[0]")
        && let Some(code) = service
            .concept
            .as_ref()
            .and_then(|c| c.coding.first())
            .and_then(|c| c.code.as_ref())
    {
        return code.0.clone();
    }
    "general".to_string()
}

#[utoipa::path(
    post,
    path = "/fhir/Schedule",
    tag = "FHIR",
    summary = "Create schedule",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_schedule(State(state): State<AppState>, Json(body): Json<Value>) -> FhirResult {
    let owner = parse_schedule_owner(&body).map_err(error_to_response)?;
    let svc = parse_service_type(&body);
    let sched = Schedule::new(owner, svc);
    let saved = ScheduleRepository::create(&state.db, &sched)
        .await
        .map_err(error_to_response)?;
    fhir_of("Schedule", saved)
}

#[utoipa::path(
    put,
    path = "/fhir/Schedule/{id}",
    tag = "FHIR",
    summary = "Update schedule",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> FhirResult {
    let mut existing = ScheduleRepository::find_by_id(&state.db, id)
        .await
        .map_err(error_to_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(operation_outcome::not_found(format!("Schedule/{id}"))),
            )
        })?;
    if body.get("actor").is_some() || body.get("owner").is_some() {
        existing.owner = parse_schedule_owner(&body).map_err(error_to_response)?;
    }
    if body.get("service_type").is_some() || body.get("serviceType").is_some() {
        existing.service_type = parse_service_type(&body);
    }
    if let Some(active) = body.get("active").and_then(sea_orm::JsonValue::as_bool) {
        existing.active = active;
    }
    existing.updated_at = chrono::Utc::now();
    let saved = ScheduleRepository::update(&state.db, &existing)
        .await
        .map_err(error_to_response)?;
    fhir_of("Schedule", saved)
}

#[utoipa::path(
    delete,
    path = "/fhir/Schedule/{id}",
    tag = "FHIR",
    summary = "Delete schedule",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    ScheduleRepository::delete(&state.db, id)
        .await
        .map_err(error_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Slot writes ──────────────────────────────────────────────────────────

fn parse_slot_status(body: &Value) -> Result<Option<SlotStatus>, Error> {
    // FHIR Slot.status: "free" | "busy" | "busy-unavailable" |
    // "busy-tentative" | "entered-in-error". PAS supports the first two
    // and maps "busy-unavailable" → BlockedOut. The value is checked against the FHIR Slot
    // status codes by the `fhir` crate; the PAS-native spellings are accepted as well.
    let Some(s) = body.get("status").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let mapped = match s.to_ascii_lowercase().as_str() {
        "free" => SlotStatus::Free,
        "busy" => SlotStatus::Busy,
        "busy-unavailable" | "blocked_out" | "blockedout" => SlotStatus::BlockedOut,
        other => {
            return Err(Error::validation(format!(
                "Slot.status {other:?} not supported (free/busy/busy-unavailable)"
            )));
        }
    };
    Ok(Some(mapped))
}

fn parse_dt(body: &Value, key: &str) -> Result<chrono::DateTime<chrono::Utc>, Error> {
    let raw = body
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::validation(format!("Slot.{key} is required")))?;
    // FHIR `instant`: RFC 3339 with a mandatory offset.
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|e| Error::validation(format!("Slot.{key} {raw:?}: {e}")))
}

fn parse_schedule_ref(body: &Value) -> Result<Uuid, Error> {
    // FHIR Slot.schedule.reference = "Schedule/<uuid>"; PAS-native
    // accepts top-level "schedule_id".
    if let Some(s) = body.get("schedule_id").and_then(|v| v.as_str()) {
        return Uuid::parse_str(s)
            .map_err(|e| Error::validation(format!("schedule_id {s:?}: {e}")));
    }
    let schedule: fhir::r5::types::Reference = body
        .get("schedule")
        .ok_or_else(|| Error::validation("Slot.schedule.reference is required"))
        .and_then(|v| fhir_element(v, "Slot.schedule"))?;
    let reference = schedule
        .reference
        .as_ref()
        .map(|r| r.0.as_str())
        .ok_or_else(|| Error::validation("Slot.schedule.reference is required"))?;
    let id_str = reference.strip_prefix("Schedule/").ok_or_else(|| {
        Error::validation(format!(
            "schedule reference must be Schedule/<uuid>, got {reference:?}"
        ))
    })?;
    Uuid::parse_str(id_str).map_err(|e| Error::validation(format!("schedule id {id_str:?}: {e}")))
}

#[utoipa::path(
    post,
    path = "/fhir/Slot",
    tag = "FHIR",
    summary = "Create slot",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn create_slot(State(state): State<AppState>, Json(body): Json<Value>) -> FhirResult {
    let schedule_id = parse_schedule_ref(&body).map_err(error_to_response)?;
    let start = parse_dt(&body, "start").map_err(error_to_response)?;
    let end = parse_dt(&body, "end").map_err(error_to_response)?;
    let mut slot = Slot::new(schedule_id, start, end);
    if let Some(status) = parse_slot_status(&body).map_err(error_to_response)? {
        slot.status = status;
    }
    let saved = SlotRepository::create(&state.db, &slot)
        .await
        .map_err(error_to_response)?;
    fhir_of("Slot", saved)
}

#[utoipa::path(
    put,
    path = "/fhir/Slot/{id}",
    tag = "FHIR",
    summary = "Update slot",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn update_slot(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> FhirResult {
    let mut existing = SlotRepository::find_by_id(&state.db, id)
        .await
        .map_err(error_to_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(operation_outcome::not_found(format!("Slot/{id}"))),
            )
        })?;
    if body.get("start").is_some() {
        existing.start_datetime = parse_dt(&body, "start").map_err(error_to_response)?;
    }
    if body.get("end").is_some() {
        existing.end_datetime = parse_dt(&body, "end").map_err(error_to_response)?;
    }
    if let Some(status) = parse_slot_status(&body).map_err(error_to_response)? {
        existing.status = status;
    }
    existing.updated_at = chrono::Utc::now();
    let saved = SlotRepository::update(&state.db, &existing)
        .await
        .map_err(error_to_response)?;
    fhir_of("Slot", saved)
}

#[utoipa::path(
    delete,
    path = "/fhir/Slot/{id}",
    tag = "FHIR",
    summary = "Delete slot",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn delete_slot(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    SlotRepository::delete(&state.db, id)
        .await
        .map_err(error_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Bundle POST with Decision #4 enforcement ─────────────────────────────

/// Bundle handler. Walks `entry[*].resource.resourceType` and
/// enforces Decision #4: any entry of type `Patient` or `Practitioner`
/// is rejected with `invalid` so callers learn the correct routing
/// (those resources are owned by person-service / worker-service and
/// must be posted directly there, never proxied through a PAS Bundle).
/// Other entries return `not-supported` for now — typed FHIR creators
/// for non-Patient/Practitioner resources are wired one-at-a-time.
#[utoipa::path(
    post,
    path = "/fhir",
    tag = "FHIR",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn post_bundle(State(_state): State<AppState>, Json(body): Json<Value>) -> FhirResult {
    let entries = body
        .get("entry")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(operation_outcome::error(
                "invalid",
                "Bundle.entry is required and must be non-empty",
            )),
        ));
    }
    let mut response_entries = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let rt = entry
            .get("resource")
            .and_then(|r| r.get("resourceType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (status, detail) = match rt {
            "" => ("400", "entry.resource.resourceType is required"),
            "Patient" => (
                "400",
                "Patient entries are owned by person-service; POST to /api/persons directly",
            ),
            "Practitioner" => (
                "400",
                "Practitioner entries are owned by worker-service; POST to /api/practitioners directly",
            ),
            _ => (
                "501",
                "typed FHIR creator for this resource is not yet wired",
            ),
        };
        response_entries.push(serde_json::json!({
            "response": {
                "status": status,
                "outcome": {
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": if status == "400" { "invalid" } else { "not-supported" },
                        "diagnostics": format!("entry[{idx}] resourceType={rt:?}: {detail}"),
                    }]
                }
            }
        }));
    }
    let any_rejected = response_entries.iter().any(|e| {
        e.get("response")
            .and_then(|r| r.get("status"))
            .and_then(|s| s.as_str())
            .map(|s| s != "201" && s != "200")
            .unwrap_or(true)
    });
    let http_status = if any_rejected {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let bundle_response = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction-response",
        "entry": response_entries,
    });
    // FhirResult is Result<Json<Value>, ...>; on a partial failure we
    // still want the structured body, so wrap into the error variant
    // with the appropriate HTTP code.
    if any_rejected {
        Err((
            http_status,
            Json(operation_outcome::error(
                "invalid",
                "Bundle contained entries that cannot be processed; see entry.response.outcome for details",
            )),
        ))
    } else {
        Ok(Json(bundle_response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_schedule_owner_accepts_fhir_actor_practitioner() {
        let id = Uuid::new_v4();
        let body = json!({
            "actor": [{ "reference": format!("Practitioner/{id}") }]
        });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Practitioner(got) => assert_eq!(got, id),
            other => panic!("expected Practitioner, got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_accepts_fhir_actor_location_as_bed() {
        let id = Uuid::new_v4();
        let body = json!({
            "actor": [{ "reference": format!("Location/{id}") }]
        });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Bed(got) => assert_eq!(got, id),
            other => panic!("expected Bed (mapped from Location), got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_accepts_pas_native_shape() {
        let id = Uuid::new_v4();
        let body = json!({ "owner": { "kind": "Room", "id": id.to_string() } });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Room(got) => assert_eq!(got, id),
            other => panic!("expected Room, got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_rejects_missing() {
        let err = parse_schedule_owner(&json!({})).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_schedule_owner_rejects_unknown_kind() {
        let id = Uuid::new_v4();
        let body = json!({ "actor": [{ "reference": format!("Organization/{id}") }] });
        let err = parse_schedule_owner(&body).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_service_type_prefers_pas_native_then_fhir_then_default() {
        assert_eq!(
            parse_service_type(&json!({ "service_type": "cardiology" })),
            "cardiology"
        );
        assert_eq!(
            parse_service_type(&json!({
                "serviceType": [{ "concept": { "coding": [{ "code": "PHYS" }] } }]
            })),
            "PHYS"
        );
        assert_eq!(parse_service_type(&json!({})), "general");
    }

    #[test]
    fn parse_slot_status_maps_fhir_strings() {
        assert_eq!(
            parse_slot_status(&json!({ "status": "free" })).unwrap(),
            Some(SlotStatus::Free)
        );
        assert_eq!(
            parse_slot_status(&json!({ "status": "busy" })).unwrap(),
            Some(SlotStatus::Busy)
        );
        assert_eq!(
            parse_slot_status(&json!({ "status": "busy-unavailable" })).unwrap(),
            Some(SlotStatus::BlockedOut)
        );
        assert!(parse_slot_status(&json!({})).unwrap().is_none());
        assert!(parse_slot_status(&json!({ "status": "garbage" })).is_err());
    }

    #[test]
    fn parse_schedule_ref_accepts_both_shapes() {
        let id = Uuid::new_v4();
        assert_eq!(
            parse_schedule_ref(&json!({ "schedule_id": id.to_string() })).unwrap(),
            id
        );
        assert_eq!(
            parse_schedule_ref(&json!({
                "schedule": { "reference": format!("Schedule/{id}") }
            }))
            .unwrap(),
            id
        );
    }

    #[test]
    fn parse_schedule_ref_rejects_bad_prefix() {
        let id = Uuid::new_v4();
        let err = parse_schedule_ref(&json!({
            "schedule": { "reference": format!("Provenance/{id}") }
        }))
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_dt_round_trips_rfc3339() {
        let v = json!({ "start": "2026-06-01T09:00:00Z" });
        let got = parse_dt(&v, "start").unwrap();
        assert_eq!(got.format("%Y%m%d%H%M%S").to_string(), "20260601090000");
    }

    #[test]
    fn parse_dt_rejects_garbage() {
        let v = json!({ "start": "not a date" });
        assert!(parse_dt(&v, "start").is_err());
    }
}

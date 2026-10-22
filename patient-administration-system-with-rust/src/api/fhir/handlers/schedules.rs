//! Part of [`super`]: schedules.

use super::*;

/// Read one element of a FHIR-shaped body as its typed R5 datatype, so a malformed one is refused with
/// the reason. Elements, not whole resources: a PUT may carry only the fields it changes.
pub(super) fn fhir_element<T: serde::de::DeserializeOwned>(
    value: &Value,
    what: &str,
) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|e| Error::validation(format!("{what}: {e}")))
}

pub(super) fn parse_schedule_owner(body: &Value) -> Result<ScheduleOwner, Error> {
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

pub(super) fn parse_service_type(body: &Value) -> String {
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

pub(super) fn parse_slot_status(body: &Value) -> Result<Option<SlotStatus>, Error> {
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

pub(super) fn parse_dt(body: &Value, key: &str) -> Result<chrono::DateTime<chrono::Utc>, Error> {
    let raw = body
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::validation(format!("Slot.{key} is required")))?;
    // FHIR `instant`: RFC 3339 with a mandatory offset.
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|e| Error::validation(format!("Slot.{key} {raw:?}: {e}")))
}

pub(super) fn parse_schedule_ref(body: &Value) -> Result<Uuid, Error> {
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

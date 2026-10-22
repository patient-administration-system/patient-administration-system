//! Part of [`super`]: reads.

use super::*;

pub(super) fn wrap_fhir(resource_type: &str, mut value: Value) -> Value {
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

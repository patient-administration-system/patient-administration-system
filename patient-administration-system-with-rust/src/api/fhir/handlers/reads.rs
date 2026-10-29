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
        Ok(Some(e)) => Ok(Json(crate::api::fhir::r5::encounter(&e))),
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
        Ok(Some(a)) => Ok(Json(crate::api::fhir::r5::appointment(&a))),
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
        Ok(Some(s)) => Ok(Json(crate::api::fhir::r5::schedule(&s))),
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
        Ok(Some(s)) => Ok(Json(crate::api::fhir::r5::slot(&s))),
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

#[utoipa::path(
    get,
    path = "/fhir/Flag",
    tag = "FHIR",
    summary = "Search flags (reasonable adjustments)",
    params(("patient" = String, Query, description = "The patient, as `Patient/<id>` or the bare id (required)")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_flags(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> FhirResult {
    let patient = q
        .get("patient")
        .map(|p| p.strip_prefix("Patient/").unwrap_or(p))
        .and_then(|p| Uuid::parse_str(p).ok())
        .ok_or_else(|| {
            error_to_response(Error::validation(
                "search by patient: ?patient=Patient/<id> is required",
            ))
        })?;
    let status = q.get("status").map(String::as_str);
    if !matches!(status, None | Some("active" | "inactive")) {
        return Err(error_to_response(Error::validation(
            "status must be active or inactive",
        )));
    }
    let all = crate::adjustments::AdjustmentService::new(state.db.clone())
        .for_patient(patient, status != Some("active"), 201)
        .await
        .map_err(error_to_response)?;
    let entries: Vec<Value> = all
        .iter()
        .filter(|a| status != Some("inactive") || a.ended_at.is_some())
        .map(|a| serde_json::json!({ "resource": a.fhir }))
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": entries.len(),
        "entry": entries,
    })))
}

#[utoipa::path(
    get,
    path = "/fhir/Observation",
    tag = "FHIR",
    summary = "Search triage categories as Observations",
    params(("encounter" = String, Query, description = "The encounter, as `Encounter/<id>` or the bare id (required)")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_observations(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> FhirResult {
    let encounter = q
        .get("encounter")
        .map(|p| p.strip_prefix("Encounter/").unwrap_or(p))
        .and_then(|p| Uuid::parse_str(p).ok())
        .ok_or_else(|| {
            error_to_response(Error::validation(
                "search by encounter: ?encounter=Encounter/<id> is required",
            ))
        })?;
    let all = crate::triage::TriageService::new(state.db.clone())
        .observations(encounter)
        .await
        .map_err(error_to_response)?;
    let entries: Vec<Value> = all
        .into_iter()
        .map(|o| serde_json::json!({ "resource": o }))
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": entries.len(),
        "entry": entries,
    })))
}

#[utoipa::path(
    get,
    path = "/fhir/SubscriptionTopic",
    tag = "FHIR",
    summary = "Search subscription topics",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_subscription_topics() -> FhirResult {
    let entries: Vec<Value> = crate::subscriptions::TOPICS
        .iter()
        .map(|(name, url)| {
            serde_json::json!({ "resource": {
                "resourceType": "SubscriptionTopic",
                "id": name,
                "url": url,
                "status": "active",
                "title": format!("Changes to {name}"),
            } })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle", "type": "searchset", "total": entries.len(), "entry": entries,
    })))
}

#[utoipa::path(
    get,
    path = "/fhir/Subscription",
    tag = "FHIR",
    summary = "Search subscriptions",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_subscriptions(State(state): State<AppState>) -> FhirResult {
    let all = crate::subscriptions::SubscriptionService::new(state.db.clone())
        .list()
        .await
        .map_err(error_to_response)?;
    let entries: Vec<Value> = all
        .iter()
        .map(|s| {
            serde_json::json!({ "resource": {
                "resourceType": "Subscription",
                "id": s.id,
                "status": if s.status == "active" { "active" } else { "off" },
                "topic": crate::subscriptions::topic_url(&s.topic),
                "channelType": {
                    "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                    "code": "rest-hook",
                },
                "endpoint": s.endpoint,
                "contentType": "application/fhir+json",
                "content": "id-only",
            } })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle", "type": "searchset", "total": entries.len(), "entry": entries,
    })))
}

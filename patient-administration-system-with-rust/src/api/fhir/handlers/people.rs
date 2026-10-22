//! Part of [`super`]: people.

use super::*;

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

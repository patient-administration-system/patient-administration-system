//! Part of [`super`]: patients.

use super::*;

#[utoipa::path(
    post,
    path = "/api/patients",
    tag = "Patients",
    summary = "Create patient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn create_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let create = PersonCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "POST /api/patients body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    // An NHS number that fails its check digit never reaches person-service (DTAC-8).
    if let Err(e) = crate::validation::check_person_identifiers(&create.fields) {
        return error_to_response(e);
    }
    match client.create(create, &ctx).await {
        Ok(person) => ok_json(person),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/patients/{id}",
    tag = "Patients",
    summary = "Get patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn get_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.get(id, &ctx).await {
        Ok(person) => ok_json(person),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    put,
    path = "/api/patients/{id}",
    tag = "Patients",
    summary = "Update patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn update_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let update = PersonCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "PUT /api/patients/{{id}} body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    if let Err(e) = crate::validation::check_person_identifiers(&update.fields) {
        return error_to_response(e);
    }
    match client.update(id, update, &ctx).await {
        Ok(person) => ok_json(person),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    delete,
    path = "/api/patients/{id}",
    tag = "Patients",
    summary = "Delete patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn delete_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.delete(id, &ctx).await {
        Ok(()) => ok(Value::Null),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/patients",
    tag = "Patients",
    summary = "Search patients",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn search_patients(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let query = q.q.unwrap_or_default();
    match client.search(&query, q.limit, &ctx).await {
        Ok(persons) => ok_json(persons),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/patients/{id}/masked",
    tag = "Patients",
    summary = "Masked patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn masked_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.masked(id, &ctx).await {
        Ok(person) => ok_json(person),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/patients/{id}/export",
    tag = "Patients",
    summary = "Export patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    // Returns just the upstream's GDPR export. It does not yet add the PAS-owned rows for the person
    // (encounters, appointments, billing, consents).
    match client.export(id, &ctx).await {
        Ok(value) => ok(value),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/patients/{id}/audit",
    tag = "Patients",
    summary = "Audit patient",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn audit_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    // Returns just the upstream's audit. It does not yet merge in the PAS-owned audit_log entries for the
    // person (admit, discharge, charge, letter, ...).
    match client.audit(id, q.limit, &ctx).await {
        Ok(entries) => ok_json(entries),
        Err(e) => error_to_response(e),
    }
}

pub(super) async fn bulk_export_format(
    state: &AppState,
    headers: &HeaderMap,
    format: &str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let client = match need_person_client(state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e).into_response(),
    };
    let ctx = user_context_from_headers(headers);
    match client.bulk_export(format, &ctx).await {
        Ok((content_type, body)) => {
            // Every patient's record leaves in one file: the read audit records it, with the count when the
            // format makes one plain (OFH-2).
            let count = export_records(format, body.as_ref());
            let mut response = (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, content_type)],
                body,
            )
                .into_response();
            response
                .extensions_mut()
                .insert(crate::api::rest::read_audit::PeopleListed(count));
            response
        }
        Err(e) => error_to_response(e).into_response(),
    }
}

/// How many records an export holds: a JSON array's length, or the lines after a CSV or TSV header. `None` for
/// XML, whose shape is the person-service's, or for anything that does not parse.
pub(super) fn export_records(format: &str, body: &[u8]) -> Option<usize> {
    match format {
        "json" => crate::api::rest::read_audit::records_in(body),
        "csv" | "tsv" => {
            let text = std::str::from_utf8(body).ok()?;
            Some(
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
                    .saturating_sub(1),
            )
        }
        _ => None,
    }
}

#[utoipa::path(
    get,
    path = "/api/patients/export.json",
    tag = "Patients",
    summary = "Export patients json",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_patients_json(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "json").await
}

#[utoipa::path(
    get,
    path = "/api/patients/export.xml",
    tag = "Patients",
    summary = "Export patients xml",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_patients_xml(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "xml").await
}

#[utoipa::path(
    get,
    path = "/api/patients/export.tsv",
    tag = "Patients",
    summary = "Export patients tsv",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_patients_tsv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "tsv").await
}

#[utoipa::path(
    get,
    path = "/api/patients/export.csv",
    tag = "Patients",
    summary = "Export patients csv",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn export_patients_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "csv").await
}

/// `POST /api/patients/import` — accepts any content-type the upstream
/// parses (`application/json`, `application/xml`, `text/csv`,
/// `text/tab-separated-values`) and forwards the body verbatim.
/// Returns the upstream's import report (as JSON) under the PAS
/// envelope so callers can integrate with the same error-handling
/// surface as the typed endpoints.
#[utoipa::path(
    post,
    path = "/api/patients/import",
    tag = "Patients",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn import_patients(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    match client.bulk_import(&content_type, body.to_vec(), &ctx).await {
        Ok(value) => ok(value),
        Err(e) => error_to_response(e),
    }
}

//! Part of [`super`]: practitioners.

use super::*;

#[utoipa::path(
    post,
    path = "/api/practitioners",
    tag = "Practitioners",
    summary = "Create practitioner",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create_practitioner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let create = WorkerCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "POST /api/practitioners body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    match client.create(create, &ctx).await {
        Ok(worker) => ok_json(worker),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/practitioners/{id}",
    tag = "Practitioners",
    summary = "Get practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.get(id, &ctx).await {
        Ok(worker) => ok_json(worker),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    put,
    path = "/api/practitioners/{id}",
    tag = "Practitioners",
    summary = "Update practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn update_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let update = WorkerCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "PUT /api/practitioners/{{id}} body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    match client.update(id, update, &ctx).await {
        Ok(worker) => ok_json(worker),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    delete,
    path = "/api/practitioners/{id}",
    tag = "Practitioners",
    summary = "Delete practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn delete_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
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
    path = "/api/practitioners",
    tag = "Practitioners",
    summary = "Search practitioners",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn search_practitioners(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let query = q.q.unwrap_or_default();
    match client.search(&query, q.limit, &ctx).await {
        Ok(workers) => ok_json(workers),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/practitioners/{id}/masked",
    tag = "Practitioners",
    summary = "Masked practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn masked_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.masked(id, &ctx).await {
        Ok(worker) => ok_json(worker),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/practitioners/{id}/export",
    tag = "Practitioners",
    summary = "Export practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn export_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.export(id, &ctx).await {
        Ok(value) => ok(value),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/practitioners/{id}/audit",
    tag = "Practitioners",
    summary = "Audit practitioner",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn audit_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.audit(id, q.limit, &ctx).await {
        Ok(entries) => ok_json(entries),
        Err(e) => error_to_response(e),
    }
}

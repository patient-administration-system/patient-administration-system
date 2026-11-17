//! REST handlers for the staff work queue (TSC-1). The person acting is the signed-in user (the verified
//! token's subject); every claim, release, reassignment and take-next is audited.

use axa::extract::{Json, Path, Query};
use axum::{extract::State, http::HeaderMap};
use serde::Deserialize;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json, request_audit_ctx};
use crate::work_queue::{Filter, WorkQueueService};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct WorkQuery {
    /// One queue: `booking`, `communication`, `operations` or `validation`.
    pub queue: Option<String>,
    /// `me` for the signed-in person's items, `none` for items nobody has, or a person's name.
    pub owner: Option<String>,
    /// Only items due before now.
    pub overdue: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_work_queue::AssignBody)]
pub struct AssignBody {
    /// The person who should have the item.
    pub to: String,
}

fn me(headers: &HeaderMap) -> Option<String> {
    request_audit_ctx(headers).user_id
}

/// `GET /api/work-items`: open and claimed work, most urgent first, then soonest due, then oldest.
#[utoipa::path(
    get,
    path = "/api/work-items",
    tag = "Work queue",
    params(crate::paging::PageQuery, WorkQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = Vec<crate::work_queue::WorkItem>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Query(q): Query<WorkQuery>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 200) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    let mut filter = Filter {
        queue: q.queue,
        overdue: q.overdue.unwrap_or(false),
        ..Filter::default()
    };
    match q.owner.as_deref() {
        None => {}
        Some("none") => filter.unclaimed = true,
        Some("me") => match me(&headers) {
            Some(who) => filter.owner = Some(who),
            None => {
                return error_to_response(Error::validation("owner=me needs a signed-in person"));
            }
        },
        Some(name) => filter.owner = Some(name.to_string()),
    }
    match WorkQueueService::new(state.db.clone())
        .list(&filter, page.rows())
        .await
    {
        Ok(v) => crate::api::rest::handlers::ok_cursor_page(&page, v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/work-items/{id}`: one work item.
#[utoipa::path(
    get,
    path = "/api/work-items/{id}",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match WorkQueueService::new(state.db.clone()).get(id).await {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/work-items/{id}/claim`: the signed-in person takes the item. `409` if someone else has it.
#[utoipa::path(
    post,
    path = "/api/work-items/{id}/claim",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let ctx = request_audit_ctx(&headers);
    let who = ctx.user_id.clone().unwrap_or_default();
    match WorkQueueService::new(state.db.clone())
        .claim(id, &who, &ctx)
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/work-items/{id}/release`: the person who has the item gives it back to its queue.
#[utoipa::path(
    post,
    path = "/api/work-items/{id}/release",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let ctx = request_audit_ctx(&headers);
    let who = ctx.user_id.clone().unwrap_or_default();
    match WorkQueueService::new(state.db.clone())
        .release(id, &who, &ctx)
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/work-items/{id}/assign`: give the item to a named person.
#[utoipa::path(
    post,
    path = "/api/work-items/{id}/assign",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = AssignBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn assign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<AssignBody>,
) -> HandlerResult {
    match WorkQueueService::new(state.db.clone())
        .assign(id, &b.to, &request_audit_ctx(&headers))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/work-queues/{queue}/next`: the signed-in person takes the next open item of the queue (most
/// urgent, then soonest due, then oldest). `404` when the queue is empty.
#[utoipa::path(
    post,
    path = "/api/work-queues/{queue}/next",
    tag = "Work queue",
    params(("queue" = String, Path, description = "`booking`, `communication`, `operations` or `validation`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn take_next(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(queue): Path<String>,
) -> HandlerResult {
    let ctx = request_audit_ctx(&headers);
    let who = ctx.user_id.clone().unwrap_or_default();
    match WorkQueueService::new(state.db.clone())
        .take_next(&queue, &who, &ctx)
        .await
    {
        Ok(Some(v)) => ok_json(v),
        Ok(None) => error_to_response(Error::not_found(format!(
            "nothing open in the {queue} queue"
        ))),
        Err(e) => error_to_response(e),
    }
}

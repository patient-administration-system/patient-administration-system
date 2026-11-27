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
    /// Only items of this priority: 1 (high) to 3 (low).
    pub priority: Option<i16>,
    /// Only items of this kind: `call`, `follow_up`, `validation`, `needs_review`, `tolerance_alert` or `unreached`.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_work_queue::AssignBody)]
pub struct AssignBody {
    /// The person who should have the item.
    pub to: String,
}

/// The person an item belongs to. A trusted service acting for someone (the UI's back end) is recorded as
/// `svc:<service> for <person>`; the item is that person's, while the audit row keeps the whole identity.
fn person_of(identity: &str) -> &str {
    match identity
        .strip_prefix("svc:")
        .and_then(|rest| rest.split_once(" for "))
    {
        Some((_, person)) if !person.trim().is_empty() => person.trim(),
        _ => identity,
    }
}

fn me(headers: &HeaderMap) -> Option<String> {
    request_audit_ctx(headers)
        .user_id
        .map(|id| person_of(&id).to_string())
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
    let kind = match q
        .kind
        .as_deref()
        .map(str::parse::<crate::work_queue::WorkKind>)
    {
        None => None,
        Some(Ok(k)) => Some(k),
        Some(Err(e)) => return error_to_response(e),
    };
    let mut filter = Filter {
        queue: q.queue,
        overdue: q.overdue.unwrap_or(false),
        priority: q.priority,
        kind,
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

/// `GET /api/work-items/reminders`: the signed-in person's items past due, and those due by the end of the
/// current working day. The reminder before an item is due is shown where the person works; nothing is sent.
#[utoipa::path(
    get,
    path = "/api/work-items/reminders",
    tag = "Work queue",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::Reminders),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn reminders(State(state): State<AppState>, headers: HeaderMap) -> HandlerResult {
    let Some(who) = me(&headers) else {
        return error_to_response(Error::validation("reminders need a signed-in person"));
    };
    match WorkQueueService::new(state.db.clone())
        .reminders(&who, state.clock.now())
        .await
    {
        Ok(v) => ok_json(v),
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
    // RCA-DECL-1: no caller is blank, which the service refuses (a named person is required).
    let who = me(&headers).unwrap_or(String::new());
    match WorkQueueService::new(state.db.clone())
        .claim(id, &who, &ctx)
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_work_queue::PhoneOutcomeBody)]
pub struct PhoneOutcomeBody {
    /// `reached`, `no_answer` or `wrong_number`.
    #[schema(value_type = crate::work_queue::PhoneOutcome)]
    pub outcome: String,
}

/// `POST /api/work-items/{id}/phone-outcome`: what came of phoning the patient for an `unreached` item
/// (TSC-8); any outcome closes it. Another kind of item is a `409`: it closes in its own list.
#[utoipa::path(
    post,
    path = "/api/work-items/{id}/phone-outcome",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = PhoneOutcomeBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkItem),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn phone_outcome(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PhoneOutcomeBody>,
) -> HandlerResult {
    let outcome = match body.outcome.parse::<crate::work_queue::PhoneOutcome>() {
        Ok(o) => o,
        Err(e) => return error_to_response(e),
    };
    let ctx = request_audit_ctx(&headers);
    // RCA-DECL-1: no caller is blank, which the service refuses (a named person is required).
    let who = me(&headers).unwrap_or(String::new());
    match WorkQueueService::new(state.db.clone())
        .record_phone_outcome(id, outcome, &who, &ctx)
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
    // RCA-DECL-1: no caller is blank, which the service refuses (a named person is required).
    let who = me(&headers).unwrap_or(String::new());
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
    // RCA-DECL-1: no caller is blank, which the service refuses (a named person is required).
    let who = me(&headers).unwrap_or(String::new());
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

#[cfg(test)]
mod tests {
    use super::person_of;

    #[test]
    fn an_item_belongs_to_the_person_a_service_acts_for() {
        assert_eq!(person_of("svc:pas-ui for nurse-7"), "nurse-7");
        assert_eq!(person_of("nurse-7"), "nurse-7");
        assert_eq!(
            person_of("svc:pas-ui"),
            "svc:pas-ui",
            "a service on its own is itself"
        );
        assert_eq!(person_of("svc:pas-ui for  "), "svc:pas-ui for  ");
    }
}

/// The verified roles of the caller (none without a verified token).
fn roles(auth: Option<&axum::Extension<crate::api::rest::user_auth::RequestAuth>>) -> Vec<String> {
    auth.map(|a| a.roles()).unwrap_or_default()
}

fn signed_in(headers: &HeaderMap) -> Result<String, Error> {
    me(headers).ok_or_else(|| Error::validation("saved views need a signed-in person"))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = handlers_work_queue::ShareBody)]
pub struct ShareBody {
    /// The role to share with, or none to stop sharing.
    pub role: Option<String>,
}

/// `POST /api/work-views`: save a named filter of the work queue (TSC-7) for the signed-in person.
#[utoipa::path(
    post,
    path = "/api/work-views",
    tag = "Work queue",
    request_body = crate::work_queue::NewView,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkView),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn save_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<crate::work_queue::NewView>,
) -> HandlerResult {
    let who = match signed_in(&headers) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    match WorkQueueService::new(state.db.clone())
        .save_view(&who, body, &request_audit_ctx(&headers))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/work-views`: the signed-in person's views and those shared with one of their roles.
#[utoipa::path(
    get,
    path = "/api/work-views",
    tag = "Work queue",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = Vec<crate::work_queue::WorkView>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list_views(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: Option<axum::Extension<crate::api::rest::user_auth::RequestAuth>>,
) -> HandlerResult {
    let who = match signed_in(&headers) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    match WorkQueueService::new(state.db.clone())
        .views_for(&who, &roles(auth.as_ref()))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/work-views/{id}/items`: the items of a view, as the signed-in person sees them; the same as
/// `GET /api/work-items` with the view's filter.
#[utoipa::path(
    get,
    path = "/api/work-views/{id}/items",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`"), crate::paging::PageQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = Vec<crate::work_queue::WorkItem>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn view_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: Option<axum::Extension<crate::api::rest::user_auth::RequestAuth>>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 200) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    let who = match signed_in(&headers) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    match WorkQueueService::new(state.db.clone())
        .view_items(id, &who, &roles(auth.as_ref()), page.rows())
        .await
    {
        Ok(v) => crate::api::rest::handlers::ok_cursor_page(&page, v),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/work-views/{id}/share`: share a view with a role, or stop sharing it. Only the person who saved
/// it; audited.
#[utoipa::path(
    put,
    path = "/api/work-views/{id}/share",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ShareBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::work_queue::WorkView),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn share_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ShareBody>,
) -> HandlerResult {
    let who = match signed_in(&headers) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    match WorkQueueService::new(state.db.clone())
        .share_view(id, &who, body.role.as_deref(), &request_audit_ctx(&headers))
        .await
    {
        Ok(v) => ok_json(v),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/work-views/{id}`: delete a view. Only the person who saved it; audited.
#[utoipa::path(
    delete,
    path = "/api/work-views/{id}",
    tag = "Work queue",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::rest::handlers_work_queue::Deleted),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn delete_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let who = match signed_in(&headers) {
        Ok(w) => w,
        Err(e) => return error_to_response(e),
    };
    match WorkQueueService::new(state.db.clone())
        .delete_view(id, &who, &request_audit_ctx(&headers))
        .await
    {
        Ok(()) => ok_json(Deleted { deleted: true }),
        Err(e) => error_to_response(e),
    }
}

/// The answer to a delete.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct Deleted {
    pub deleted: bool,
}

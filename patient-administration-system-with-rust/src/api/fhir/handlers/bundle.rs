//! Part of [`super`]: bundle.

use std::collections::HashMap;

use axum::Extension;
use sea_orm::{ConnectionTrait, TransactionTrait};

use super::*;
use crate::api::rest::handlers::request_audit_ctx;
use crate::api::rest::user_auth::RequestAuth;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;

/// The server's CapabilityStatement: which resources and interactions it supports (NEW-21).
#[utoipa::path(
    get,
    path = "/fhir/metadata",
    tag = "FHIR",
    summary = "CapabilityStatement",
    responses((status = 200, description = "The FHIR R5 CapabilityStatement")),
)]
pub async fn metadata() -> Json<Value> {
    Json(crate::api::fhir::capability::statement())
}

/// What one Bundle entry would write, once it has been checked.
enum Write {
    Schedule(Schedule),
    /// The body stays as JSON until the Schedule it may refer to has its id.
    Slot(Value),
}

/// One entry refused, with the HTTP status it would have had on its own.
struct Refusal {
    status: StatusCode,
    code: &'static str,
    detail: String,
}

impl Refusal {
    fn new(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }

    fn of(err: Error) -> Self {
        let (status, code) = match &err {
            Error::NotFound(_) => (StatusCode::NOT_FOUND, "not-found"),
            Error::Validation(_) | Error::Fhir(_) => (StatusCode::BAD_REQUEST, "invalid"),
            Error::InvalidStateTransition(_) | Error::Conflict(_) => {
                (StatusCode::CONFLICT, "conflict")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "exception"),
        };
        Self::new(status, code, err.to_string())
    }

    fn at(self, index: usize) -> Self {
        Self {
            detail: format!("entry[{index}]: {}", self.detail),
            ..self
        }
    }

    fn response(&self) -> Value {
        serde_json::json!({
            "response": {
                "status": status_line(self.status),
                "outcome": operation_outcome::error(self.code, self.detail.clone()),
            }
        })
    }
}

fn status_line(status: StatusCode) -> String {
    format!(
        "{} {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    )
}

/// What may be done with this entry, before anything is read or written. Pure apart from `auth`.
fn check_entry(entry: &Value, auth: Option<&RequestAuth>) -> Result<Write, Refusal> {
    let bad = |detail: String| Refusal::new(StatusCode::BAD_REQUEST, "invalid", detail);
    let resource = entry.get("resource").unwrap_or(&Value::Null);
    let kind = resource
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        "" => return Err(bad("entry.resource.resourceType is required".into())),
        "Patient" => {
            return Err(bad(
                "Patient entries are owned by person-service; POST to /fhir/Patient directly"
                    .into(),
            ));
        }
        "Practitioner" => {
            return Err(bad(
                "Practitioner entries are owned by worker-service; POST to /fhir/Practitioner directly"
                    .into(),
            ));
        }
        "Schedule" | "Slot" => {}
        other => {
            return Err(Refusal::new(
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
                format!("a Bundle can write Schedule and Slot; {other} is not supported"),
            ));
        }
    }
    let request = entry.get("request").unwrap_or(&Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let url = request.get("url").and_then(Value::as_str).unwrap_or("");
    if method.is_empty() || url.is_empty() {
        return Err(bad("entry.request.method and url are required".into()));
    }
    if method != "POST" {
        return Err(Refusal::new(
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            format!("a Bundle entry can only create; use {method} /fhir/{url} directly"),
        ));
    }
    if url != kind {
        return Err(bad(format!(
            "entry.request.url {url:?} must be {kind:?}, the type of the resource"
        )));
    }
    if request.get("ifNoneExist").is_some() {
        return Err(Refusal::new(
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            "conditional create (ifNoneExist) is not supported",
        ));
    }
    // The same answer `POST /fhir/Slot` would give this caller: a Bundle must not reach further.
    if let Some(auth) = auth
        && !auth.allows(&axum::http::Method::POST, &format!("/fhir/{kind}"))
    {
        return Err(Refusal::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("your account is not permitted to create {kind}"),
        ));
    }
    if kind == "Schedule" {
        schedule_from(resource)
            .map(Write::Schedule)
            .map_err(Refusal::of)
    } else {
        Ok(Write::Slot(resource.clone()))
    }
}

/// Points `Slot.schedule` at the Schedule a `urn:uuid:` reference stands for. Any other reference stays.
fn resolve_schedule(body: &mut Value, ids: &HashMap<String, Uuid>) -> Result<(), Error> {
    let Some(reference) = body.pointer_mut("/schedule/reference") else {
        return Ok(());
    };
    let Some(text) = reference.as_str().filter(|r| r.starts_with("urn:uuid:")) else {
        return Ok(());
    };
    let id = ids.get(text).ok_or_else(|| {
        Error::validation(format!(
            "no Schedule entry in this Bundle has fullUrl {text}"
        ))
    })?;
    *reference = Value::String(format!("Schedule/{id}"));
    Ok(())
}

async fn save_schedule<C: ConnectionTrait>(
    conn: &C,
    schedule: &Schedule,
    ctx: &UserContext,
) -> Result<String, Error> {
    let saved = ScheduleRepository::create(conn, schedule).await?;
    let value = serde_json::to_value(&saved).unwrap_or_default();
    AuditLogRepository::log(
        conn,
        "schedule",
        saved.id,
        "create",
        None,
        Some(value.clone()),
        ctx,
    )
    .await?;
    OutboxRepository::publish(conn, "ScheduleCreated", &value).await?;
    Ok(format!("Schedule/{}", saved.id))
}

/// `schedules` has a foreign key, and a Slot naming one that is not there is the caller's mistake, not a 500.
async fn save_slot<C: ConnectionTrait>(
    conn: &C,
    body: &Value,
    ctx: &UserContext,
) -> Result<String, Error> {
    let slot = slot_from(body)?;
    if ScheduleRepository::find_by_id(conn, slot.schedule_id)
        .await?
        .is_none()
    {
        return Err(Error::validation(format!(
            "Slot.schedule: Schedule/{} does not exist",
            slot.schedule_id
        )));
    }
    let saved = SlotRepository::create(conn, &slot).await?;
    let value = serde_json::to_value(&saved).unwrap_or_default();
    AuditLogRepository::log(
        conn,
        "slot",
        saved.id,
        "create",
        None,
        Some(value.clone()),
        ctx,
    )
    .await?;
    OutboxRepository::publish(conn, "SlotCreated", &value).await?;
    Ok(format!("Slot/{}", saved.id))
}

fn created(location: String) -> Value {
    serde_json::json!({
        "response": { "status": status_line(StatusCode::CREATED), "location": location }
    })
}

/// A refused transaction: what is wrong with every entry, under the status of the first.
fn refuse_all(refusals: Vec<Refusal>) -> (StatusCode, Json<OperationOutcome>) {
    let status = refusals
        .first()
        .map_or(StatusCode::BAD_REQUEST, |r| r.status);
    let code = refusals.first().map_or("invalid", |r| r.code);
    let detail = refusals
        .iter()
        .map(|r| r.detail.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    (status, Json(operation_outcome::error(code, detail)))
}

/// All or nothing, in one database transaction. Schedules go first, so that a Slot can name one by the
/// `urn:uuid:` `fullUrl` of its entry wherever the entries stand; responses keep the order of the request.
async fn run_transaction(
    state: &AppState,
    entries: &[Value],
    auth: Option<&RequestAuth>,
    ctx: &UserContext,
) -> FhirResult {
    let mut writes = Vec::with_capacity(entries.len());
    let mut refusals = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match check_entry(entry, auth) {
            Ok(write) => writes.push((index, write)),
            Err(refusal) => refusals.push(refusal.at(index)),
        }
    }
    let mut ids = HashMap::new();
    for (index, write) in &writes {
        if let Write::Schedule(schedule) = write
            && let Some(url) = entries[*index].get("fullUrl").and_then(Value::as_str)
            && ids.insert(url.to_string(), schedule.id).is_some()
        {
            refusals.push(
                Refusal::new(
                    StatusCode::BAD_REQUEST,
                    "invalid",
                    format!("fullUrl {url} is used twice"),
                )
                .at(*index),
            );
        }
    }
    for (index, write) in &mut writes {
        if let Write::Slot(body) = write
            && let Err(e) = resolve_schedule(body, &ids).and_then(|()| slot_from(body).map(drop))
        {
            refusals.push(Refusal::of(e).at(*index));
        }
    }
    if !refusals.is_empty() {
        return Err(refuse_all(refusals));
    }
    writes.sort_by_key(|(_, write)| matches!(write, Write::Slot(_)));
    let txn = state
        .db
        .begin()
        .await
        .map_err(|e| error_to_response(Error::Database(e)))?;
    let mut responses = vec![Value::Null; entries.len()];
    for (index, write) in &writes {
        let saved = match write {
            Write::Schedule(schedule) => save_schedule(&txn, schedule, ctx).await,
            Write::Slot(body) => save_slot(&txn, body, ctx).await,
        };
        match saved {
            Ok(location) => responses[*index] = created(location),
            // Dropping `txn` rolls back whatever came before.
            Err(e) => return Err(refuse_all(vec![Refusal::of(e).at(*index)])),
        }
    }
    txn.commit()
        .await
        .map_err(|e| error_to_response(Error::Database(e)))?;
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction-response",
        "entry": responses,
    })))
}

/// Each entry alone, in its own database transaction, and each with its own answer. A batch's entries may
/// not depend on one another, so a `urn:uuid:` reference is refused. HTTP 200 whatever the entries did.
async fn run_batch(
    state: &AppState,
    entries: &[Value],
    auth: Option<&RequestAuth>,
    ctx: &UserContext,
) -> FhirResult {
    let mut responses = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let done = match check_entry(entry, auth) {
            Err(refusal) => Err(refusal),
            Ok(write) => run_one(state, &write, ctx).await.map_err(Refusal::of),
        };
        responses.push(match done {
            Ok(location) => created(location),
            Err(refusal) => refusal.at(index).response(),
        });
    }
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle",
        "type": "batch-response",
        "entry": responses,
    })))
}

async fn run_one(state: &AppState, write: &Write, ctx: &UserContext) -> Result<String, Error> {
    let mut slot;
    let txn = state.db.begin().await.map_err(Error::Database)?;
    let location = match write {
        Write::Schedule(schedule) => save_schedule(&txn, schedule, ctx).await?,
        Write::Slot(body) => {
            slot = body.clone();
            resolve_schedule(&mut slot, &HashMap::new())?;
            save_slot(&txn, &slot, ctx).await?
        }
    };
    txn.commit().await.map_err(Error::Database)?;
    Ok(location)
}

/// Writes a FHIR `transaction` or `batch` Bundle.
///
/// An entry can create a Schedule or a Slot, which are the records this service owns and can write from
/// FHIR. Each is audited and publishes an event in the transaction that writes it. A `Patient` or
/// `Practitioner` entry is refused (Decision #4): `person-service` and `worker-service` own them, and a
/// write there cannot share this database transaction. Any other type, method or `ifNoneExist` answers
/// `501`. Each entry is authorized as if it had been posted to `/fhir/{type}` by the same caller.
#[utoipa::path(
    post,
    path = "/fhir",
    tag = "FHIR",
    responses(
        (status = 200, description = "A transaction-response or batch-response Bundle"),
        (status = "4XX", description = "Error; an OperationOutcome. A transaction writes nothing"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn post_bundle(
    State(state): State<AppState>,
    auth: Option<Extension<RequestAuth>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let invalid = |detail: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(operation_outcome::error("invalid", detail)),
        )
    };
    if body.get("resourceType").and_then(Value::as_str) != Some("Bundle") {
        return Err(invalid("the body must be a Bundle"));
    }
    let entries = body
        .get("entry")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| invalid("Bundle.entry is required and must be non-empty"))?;
    let auth = auth.as_ref().map(|Extension(auth)| auth);
    let ctx = request_audit_ctx(&headers);
    match body.get("type").and_then(Value::as_str) {
        Some("transaction") => run_transaction(&state, entries, auth, &ctx).await,
        Some("batch") => run_batch(&state, entries, auth, &ctx).await,
        _ => Err(invalid("Bundle.type must be transaction or batch")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(resource: Value, method: &str, url: &str) -> Value {
        json!({ "resource": resource, "request": { "method": method, "url": url } })
    }

    fn schedule() -> Value {
        json!({
            "resourceType": "Schedule",
            "actor": [{ "reference": format!("Practitioner/{}", Uuid::new_v4()) }],
        })
    }

    fn refused(entry: &Value) -> Refusal {
        match check_entry(entry, None) {
            Err(refusal) => refusal,
            Ok(_) => panic!("{entry} was accepted"),
        }
    }

    #[test]
    fn schedule_and_slot_entries_are_accepted() {
        assert!(matches!(
            check_entry(&entry(schedule(), "POST", "Schedule"), None),
            Ok(Write::Schedule(_))
        ));
        let slot = json!({ "resourceType": "Slot" });
        assert!(matches!(
            check_entry(&entry(slot, "POST", "Slot"), None),
            Ok(Write::Slot(_))
        ));
    }

    #[test]
    fn identity_stays_with_its_owners() {
        for kind in ["Patient", "Practitioner"] {
            let r = refused(&entry(json!({ "resourceType": kind }), "POST", kind));
            assert_eq!(r.status, StatusCode::BAD_REQUEST);
            assert!(r.detail.contains("owned by"), "{}", r.detail);
        }
    }

    #[test]
    fn what_cannot_be_written_says_so() {
        let cases = [
            entry(json!({ "resourceType": "Encounter" }), "POST", "Encounter"),
            entry(schedule(), "PUT", "Schedule/1"),
            entry(schedule(), "DELETE", "Schedule/1"),
            json!({
                "resource": schedule(),
                "request": { "method": "POST", "url": "Schedule", "ifNoneExist": "actor=x" },
            }),
        ];
        for case in &cases {
            assert_eq!(refused(case).status, StatusCode::NOT_IMPLEMENTED, "{case}");
        }
    }

    #[test]
    fn an_entry_must_say_what_it_is_doing() {
        assert_eq!(refused(&json!({})).status, StatusCode::BAD_REQUEST);
        assert_eq!(
            refused(&json!({ "resource": schedule() })).status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refused(&entry(schedule(), "POST", "Slot")).status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn a_reference_to_an_entry_becomes_its_id() {
        let id = Uuid::new_v4();
        let ids = HashMap::from([("urn:uuid:a".to_string(), id)]);
        let mut body = json!({ "schedule": { "reference": "urn:uuid:a" } });
        resolve_schedule(&mut body, &ids).unwrap();
        assert_eq!(body["schedule"]["reference"], format!("Schedule/{id}"));

        let mut unknown = json!({ "schedule": { "reference": "urn:uuid:b" } });
        assert!(resolve_schedule(&mut unknown, &ids).is_err());

        let mut existing = json!({ "schedule": { "reference": format!("Schedule/{id}") } });
        resolve_schedule(&mut existing, &HashMap::new()).unwrap();
        assert_eq!(existing["schedule"]["reference"], format!("Schedule/{id}"));
    }

    #[test]
    fn a_transaction_is_refused_with_every_problem_under_the_first_status() {
        let (status, Json(outcome)) = refuse_all(vec![
            Refusal::new(StatusCode::BAD_REQUEST, "invalid", "one").at(0),
            Refusal::new(StatusCode::NOT_IMPLEMENTED, "not-supported", "two").at(2),
        ]);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let text = serde_json::to_string(&outcome).unwrap();
        assert!(
            text.contains("entry[0]: one") && text.contains("entry[2]: two"),
            "{text}"
        );
    }
}

//! Part of [`super`]: bundle.

use crate::db::sql::Transact;
use std::collections::HashMap;

use axum::Extension;
use sea_orm::DatabaseTransaction;

use super::*;
use crate::adt::{AdmissionResult, AdtService};
use crate::api::rest::handlers::request_audit_ctx;
use crate::api::rest::user_auth::RequestAuth;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::appointment::Appointment;
use crate::models::coverage::Coverage;
use crate::scheduling::SchedulingService;

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

/// What one Bundle entry would write, once it has been checked. The ones that may refer to another entry
/// stay as JSON until that entry has its id.
enum Write {
    Schedule(Schedule),
    Slot(Value),
    Coverage(Box<Coverage>),
    Appointment(Value),
    Encounter(Value),
}

impl Write {
    /// The order to write in: what can be referred to comes before what refers to it.
    fn rank(&self) -> u8 {
        match self {
            Self::Schedule(_) => 0,
            Self::Slot(_) => 1,
            Self::Coverage(_) => 2,
            Self::Appointment(_) => 3,
            Self::Encounter(_) => 4,
        }
    }
}

/// A [`Write`] with its references resolved, ready to save.
enum Plan {
    Schedule(Schedule),
    Slot(Slot),
    Coverage(Box<Coverage>),
    Appointment { slot_id: Uuid, person_id: Uuid },
    Encounter { person_id: Uuid, bed_id: Uuid },
}

impl Plan {
    /// What another entry would call this one. Only these two can be referred to.
    fn reference(&self) -> Option<String> {
        match self {
            Self::Schedule(s) => Some(format!("Schedule/{}", s.id)),
            Self::Slot(s) => Some(format!("Slot/{}", s.id)),
            _ => None,
        }
    }
}

/// What to do after the commit, as the REST routes do: the outbox row is already written.
enum After {
    Nothing,
    Booked(Box<Appointment>),
    Admitted(Box<AdmissionResult>),
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
        "Schedule" | "Slot" | "Coverage" | "Appointment" | "Encounter" => {}
        other => {
            return Err(Refusal::new(
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
                format!(
                    "a Bundle can write Schedule, Slot, Coverage, Appointment and Encounter; \
                     {other} is not supported"
                ),
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
    match kind {
        "Schedule" => schedule_from(resource)
            .map(Write::Schedule)
            .map_err(Refusal::of),
        "Slot" => Ok(Write::Slot(resource.clone())),
        "Coverage" => coverage_from(resource)
            .map(|c| Write::Coverage(Box::new(c)))
            .map_err(Refusal::of),
        "Appointment" => Ok(Write::Appointment(resource.clone())),
        _ if is_inpatient(resource) => Ok(Write::Encounter(resource.clone())),
        _ => Err(Refusal::new(
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            "a Bundle can admit an inpatient (Encounter.class IMP); use the REST API for other encounters",
        )),
    }
}

/// Replaces a `urn:uuid:` reference with what it stands for in this Bundle. Any other reference stays.
fn resolve(body: &mut Value, pointer: &str, refs: &HashMap<String, String>) -> Result<(), Error> {
    let Some(reference) = body.pointer_mut(pointer) else {
        return Ok(());
    };
    let Some(text) = reference.as_str().filter(|r| r.starts_with("urn:uuid:")) else {
        return Ok(());
    };
    let target = refs.get(text).ok_or_else(|| {
        Error::validation(format!(
            "no entry that can be referred to has fullUrl {text}"
        ))
    })?;
    *reference = Value::String(target.clone());
    Ok(())
}

fn plan_of(write: &Write, refs: &HashMap<String, String>) -> Result<Plan, Error> {
    Ok(match write {
        Write::Schedule(schedule) => Plan::Schedule(schedule.clone()),
        Write::Coverage(coverage) => Plan::Coverage(coverage.clone()),
        Write::Slot(body) => {
            let mut body = body.clone();
            resolve(&mut body, "/schedule/reference", refs)?;
            Plan::Slot(slot_from(&body)?)
        }
        Write::Appointment(body) => {
            let mut body = body.clone();
            resolve(&mut body, "/slot/0/reference", refs)?;
            let (slot_id, person_id) = booking_from(&body)?;
            Plan::Appointment { slot_id, person_id }
        }
        Write::Encounter(body) => {
            let (person_id, bed_id) = admission_from(body)?;
            Plan::Encounter { person_id, bed_id }
        }
    })
}

/// Writes one entry, its audit row and its outbox event. `Appointment` and `Encounter` go through the
/// services the REST routes use, so a Bundle meets the same rules: a slot must be free and a bed available.
async fn save(
    txn: &DatabaseTransaction,
    plan: &Plan,
    ctx: &UserContext,
) -> Result<(String, After), Error> {
    match plan {
        Plan::Schedule(schedule) => {
            let saved = save_schedule(txn, schedule, ctx).await?;
            Ok((format!("Schedule/{}", saved.id), After::Nothing))
        }
        Plan::Slot(slot) => {
            let saved = save_slot(txn, slot, ctx).await?;
            Ok((format!("Slot/{}", saved.id), After::Nothing))
        }
        Plan::Coverage(coverage) => {
            let saved = CoverageRepository::create(txn, coverage).await?;
            AuditLogRepository::log(
                txn,
                "coverage",
                saved.id,
                "create",
                None,
                Some(serde_json::to_value(&saved).unwrap_or_default()),
                ctx,
            )
            .await?;
            OutboxRepository::publish(
                txn,
                "CoverageCreated",
                &serde_json::json!({ "coverage_id": saved.id, "person_id": saved.person_id }),
            )
            .await?;
            Ok((format!("Coverage/{}", saved.id), After::Nothing))
        }
        Plan::Appointment { slot_id, person_id } => {
            let booked = SchedulingService::book_tx(txn, *slot_id, *person_id, ctx).await?;
            Ok((
                format!("Appointment/{}", booked.id),
                After::Booked(Box::new(booked)),
            ))
        }
        Plan::Encounter { person_id, bed_id } => {
            let admitted = AdtService::admit_tx(txn, *person_id, *bed_id, ctx).await?;
            Ok((
                format!("Encounter/{}", admitted.encounter.id),
                After::Admitted(Box::new(admitted)),
            ))
        }
    }
}

async fn after_commit(state: &AppState, after: After) {
    match after {
        After::Nothing => {}
        After::Booked(appointment) => state.scheduling.publish_booked(&appointment).await,
        After::Admitted(admission) => state.adt.publish_admitted(&admission).await,
    }
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

/// All or nothing, in one database transaction. Entries are written in the order of [`Write::rank`], so
/// that one can name another by the `urn:uuid:` `fullUrl` of its entry wherever the two stand; responses
/// keep the order of the request.
async fn run_transaction(
    state: &AppState,
    entries: &[Value],
    auth: Option<&RequestAuth>,
    ctx: &UserContext,
) -> FhirResult {
    let mut writes = Vec::with_capacity(entries.len());
    let mut refused = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match check_entry(entry, auth) {
            Ok(write) => writes.push((index, write)),
            Err(refusal) => refused.push((index, refusal.at(index))),
        }
    }
    writes.sort_by_key(|(_, write)| write.rank());
    let mut refs = HashMap::new();
    let mut plans = Vec::with_capacity(writes.len());
    for (index, write) in &writes {
        match plan_of(write, &refs) {
            Ok(plan) => {
                if let (Some(url), Some(reference)) = (
                    entries[*index].get("fullUrl").and_then(Value::as_str),
                    plan.reference(),
                ) && refs.insert(url.to_string(), reference).is_some()
                {
                    let detail = format!("fullUrl {url} is used twice");
                    let refusal = Refusal::new(StatusCode::BAD_REQUEST, "invalid", detail);
                    refused.push((*index, refusal.at(*index)));
                }
                plans.push((*index, plan));
            }
            Err(e) => refused.push((*index, Refusal::of(e).at(*index))),
        }
    }
    if !refused.is_empty() {
        refused.sort_by_key(|(index, _)| *index);
        return Err(refuse_all(refused.into_iter().map(|(_, r)| r).collect()));
    }
    let txn = state.db.begin_txn().await.map_err(error_to_response)?;
    let mut responses = vec![Value::Null; entries.len()];
    let mut afters = Vec::with_capacity(plans.len());
    for (index, plan) in &plans {
        match save(&txn, plan, ctx).await {
            Ok((location, after)) => {
                responses[*index] = created(location);
                afters.push(after);
            }
            // Dropping `txn` rolls back whatever came before.
            Err(e) => return Err(refuse_all(vec![Refusal::of(e).at(*index)])),
        }
    }
    txn.commit()
        .await
        .map_err(|e| error_to_response(Error::Database(e)))?;
    for after in afters {
        after_commit(state, after).await;
    }
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
    let plan = plan_of(write, &HashMap::new())?;
    let txn = state.db.begin_txn().await?;
    let (location, after) = save(&txn, &plan, ctx).await?;
    txn.commit().await.map_err(Error::Database)?;
    after_commit(state, after).await;
    Ok(location)
}

/// Writes a FHIR `transaction` or `batch` Bundle.
///
/// An entry can create a Schedule, a Slot or a Coverage, book an Appointment into a free Slot, or admit an
/// inpatient Encounter to an available bed. Bookings and admissions meet the rules of the REST routes, and
/// each write is audited and publishes an event in the transaction that writes it. A `Patient` or
/// `Practitioner` entry is refused (Decision #4): `person-service` and `worker-service` own them, and a
/// write there cannot share this database transaction. Any other type or method, `ifNoneExist`, and an
/// Encounter that is not inpatient answer `501`. Each entry is authorized as if it had been posted to
/// `/fhir/{type}` by the same caller.
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
            entry(
                json!({ "resourceType": "Observation" }),
                "POST",
                "Observation",
            ),
            entry(json!({ "resourceType": "Encounter" }), "POST", "Encounter"),
            entry(
                json!({
                    "resourceType": "Encounter",
                    "class": [{ "coding": [{ "code": "EMER" }] }],
                }),
                "POST",
                "Encounter",
            ),
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
    fn a_reference_to_an_entry_becomes_what_it_stands_for() {
        let target = format!("Schedule/{}", Uuid::new_v4());
        let refs = HashMap::from([("urn:uuid:a".to_string(), target.clone())]);
        let pointer = "/schedule/reference";
        let mut body = json!({ "schedule": { "reference": "urn:uuid:a" } });
        resolve(&mut body, pointer, &refs).unwrap();
        assert_eq!(body["schedule"]["reference"], target);

        let mut unknown = json!({ "schedule": { "reference": "urn:uuid:b" } });
        assert!(resolve(&mut unknown, pointer, &refs).is_err());

        let mut existing = json!({ "schedule": { "reference": target } });
        resolve(&mut existing, pointer, &HashMap::new()).unwrap();
        assert_eq!(existing["schedule"]["reference"], target);
    }

    #[test]
    fn a_coverage_read_back_from_fhir_writes_the_same_coverage() {
        let mut c = Coverage::new(Uuid::new_v4(), "Aetna", "POL-1");
        c.payor_identifier = Some("EIN-9".into());
        c.group_number = Some("G-7".into());
        c.subscriber_id = Some(Uuid::new_v4());
        c.relationship = "spouse".into();
        c.end_date = c.start_date.checked_add_days(chrono::Days::new(30));
        let again = coverage_from(&crate::api::fhir::r5::coverage(&c)).unwrap();
        assert_eq!(again.person_id, c.person_id);
        assert_eq!(again.payor_name, c.payor_name);
        assert_eq!(again.payor_identifier, c.payor_identifier);
        assert_eq!(again.policy_number, c.policy_number);
        assert_eq!(again.group_number, c.group_number);
        assert_eq!(again.subscriber_id, c.subscriber_id);
        assert_eq!(again.relationship, c.relationship);
        assert_eq!(again.start_date, c.start_date);
        assert_eq!(again.end_date, c.end_date);
    }

    #[test]
    fn what_can_be_referred_to_is_written_first() {
        let slot = Write::Slot(json!({}));
        let booking = Write::Appointment(json!({}));
        let schedule = schedule_from(&schedule()).map(Write::Schedule).unwrap();
        assert!(schedule.rank() < slot.rank() && slot.rank() < booking.rank());
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

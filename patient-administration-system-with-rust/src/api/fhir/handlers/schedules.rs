//! Part of [`super`]: schedules.

use super::*;
use crate::api::rest::handlers::request_audit_ctx;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::db::sql::{Transact, Txn};

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

/// A new Schedule from a FHIR body, for `POST /fhir/Schedule` and a Bundle entry alike.
pub(super) fn schedule_from(body: &Value) -> Result<Schedule, Error> {
    Ok(Schedule::new(
        parse_schedule_owner(body)?,
        parse_service_type(body),
    ))
}

/// A new Slot from a FHIR body, for `POST /fhir/Slot` and a Bundle entry alike.
pub(super) fn slot_from(body: &Value) -> Result<Slot, Error> {
    let schedule_id = parse_schedule_ref(body)?;
    let start = parse_dt(body, "start")?;
    let end = parse_dt(body, "end")?;
    if end <= start {
        return Err(Error::validation("Slot.end must be after Slot.start"));
    }
    let mut slot = Slot::new(schedule_id, start, end);
    if let Some(status) = parse_slot_status(body)? {
        slot.status = status;
    }
    Ok(slot)
}

fn audit_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value)
        .map_err(|e| Error::internal(format!("audit value could not be serialized: {e}")))
}

/// The audit row and the event of one write to a `kind` (`Schedule` or `Slot`), in the transaction that
/// writes it (NFR-1, NFR-2). `before` and `after` say which write it was: a create has nothing before it
/// and a delete nothing after, and the event carries the record as it now stands or last stood.
async fn record(
    txn: &Txn,
    kind: &str,
    id: Uuid,
    before: Option<Value>,
    after: Option<Value>,
    ctx: &UserContext,
) -> Result<(), Error> {
    let (action, happened) = match (&before, &after) {
        (None, _) => ("create", "Created"),
        (Some(_), Some(_)) => ("update", "Updated"),
        (Some(_), None) => ("delete", "Deleted"),
    };
    let event = after.clone().or_else(|| before.clone()).unwrap_or_default();
    AuditLogRepository::log(txn, &kind.to_lowercase(), id, action, before, after, ctx).await?;
    OutboxRepository::publish(txn, &format!("{kind}{happened}"), &event).await
}

/// Creates a Schedule with its audit row and event, for `POST /fhir/Schedule` and a Bundle entry alike.
pub(super) async fn save_schedule(
    txn: &Txn,
    schedule: &Schedule,
    ctx: &UserContext,
) -> Result<Schedule, Error> {
    let saved = ScheduleRepository::create(txn, schedule).await?;
    record(
        txn,
        "Schedule",
        saved.id,
        None,
        Some(audit_value(&saved)?),
        ctx,
    )
    .await?;
    Ok(saved)
}

/// Creates a Slot with its audit row and event, for `POST /fhir/Slot` and a Bundle entry alike.
pub(super) async fn save_slot(txn: &Txn, slot: &Slot, ctx: &UserContext) -> Result<Slot, Error> {
    // A foreign key would catch this as a 500, and a Slot naming no Schedule is the caller's mistake.
    if ScheduleRepository::find_by_id(txn, slot.schedule_id)
        .await?
        .is_none()
    {
        return Err(Error::validation(format!(
            "Slot.schedule: Schedule/{} does not exist",
            slot.schedule_id
        )));
    }
    let saved = SlotRepository::create(txn, slot).await?;
    record(txn, "Slot", saved.id, None, Some(audit_value(&saved)?), ctx).await?;
    Ok(saved)
}

/// The id in a reference such as `Patient/<uuid>`.
fn reference_id(body: &Value, pointer: &str, kind: &str, what: &str) -> Result<Uuid, Error> {
    let reference = body
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::validation(format!("{what} is required")))?;
    let id = reference
        .strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| {
            Error::validation(format!("{what} must be {kind}/<uuid>, got {reference:?}"))
        })?;
    Uuid::parse_str(id).map_err(|e| Error::validation(format!("{what} {id:?}: {e}")))
}

fn fhir_date(body: &Value, pointer: &str, what: &str) -> Result<Option<jiff::civil::Date>, Error> {
    let Some(raw) = body.pointer(pointer).and_then(Value::as_str) else {
        return Ok(None);
    };
    // A FHIR dateTime may carry a time; a coverage period is in whole days.
    let day = raw.get(..10).unwrap_or(raw);
    day.parse::<jiff::civil::Date>()
        .map(Some)
        .map_err(|e| Error::validation(format!("{what} {raw:?}: {e}")))
}

/// A new Coverage from an R5 `Coverage`, the reverse of `r5::coverage`.
pub(super) fn coverage_from(body: &Value) -> Result<Coverage, Error> {
    let text = |pointer: &str| {
        body.pointer(pointer)
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
    };
    let person_id = reference_id(
        body,
        "/beneficiary/reference",
        "Patient",
        "Coverage.beneficiary",
    )?;
    let status = match text("/status") {
        Some("active") => CoverageStatus::Active,
        Some("cancelled") => CoverageStatus::Cancelled,
        Some("draft") => CoverageStatus::Draft,
        Some("entered-in-error") => CoverageStatus::EnteredInError,
        other => return Err(Error::validation(format!("Coverage.status {other:?}"))),
    };
    let kind = match text("/kind") {
        Some("insurance") => CoverageKind::Insurance,
        Some("self-pay") => CoverageKind::SelfPay,
        Some("other") => CoverageKind::Other,
        other => return Err(Error::validation(format!("Coverage.kind {other:?}"))),
    };
    let payor = match (text("/insurer/display"), kind) {
        (Some(name), _) => name,
        (None, CoverageKind::SelfPay) => "Self-pay",
        (None, _) => return Err(Error::validation("Coverage.insurer.display is required")),
    };
    let mut coverage = Coverage::new(person_id, payor, text("/identifier/0/value").unwrap_or(""));
    coverage.status = status;
    coverage.kind = kind;
    coverage.payor_identifier = text("/insurer/identifier/value").map(String::from);
    coverage.group_number = text("/class/0/value/value").map(String::from);
    if body.pointer("/subscriber/reference").is_some() {
        coverage.subscriber_id = Some(reference_id(
            body,
            "/subscriber/reference",
            "Patient",
            "Coverage.subscriber",
        )?);
    }
    if let Some(relationship) = text("/relationship/coding/0/code").or(text("/relationship/text")) {
        coverage.relationship = relationship.to_string();
    }
    if let Some(start) = fhir_date(body, "/period/start", "Coverage.period.start")? {
        coverage.start_date = start;
    }
    coverage.end_date = fhir_date(body, "/period/end", "Coverage.period.end")?;
    if coverage
        .end_date
        .is_some_and(|end| end < coverage.start_date)
    {
        return Err(Error::validation(
            "Coverage.period.end must not be before its start",
        ));
    }
    Ok(coverage)
}

/// The slot and the patient of a booking, from an R5 `Appointment`. The times come from the slot.
pub(super) fn booking_from(body: &Value) -> Result<(Uuid, Uuid), Error> {
    let status = body.get("status").and_then(Value::as_str);
    if status != Some("booked") {
        return Err(Error::validation(format!(
            "Appointment.status must be booked, got {status:?}"
        )));
    }
    let slots = body
        .get("slot")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if slots != 1 {
        return Err(Error::validation(format!(
            "Appointment.slot must name exactly one Slot, got {slots}"
        )));
    }
    Ok((
        reference_id(body, "/slot/0/reference", "Slot", "Appointment.slot")?,
        reference_id(body, "/subject/reference", "Patient", "Appointment.subject")?,
    ))
}

/// Does this R5 `Encounter` say inpatient? Only an admission has a route into the PAS from here.
pub(super) fn is_inpatient(body: &Value) -> bool {
    body.pointer("/class/0/coding")
        .and_then(Value::as_array)
        .is_some_and(|codings| {
            codings
                .iter()
                .any(|c| c.get("code").and_then(Value::as_str) == Some("IMP"))
        })
}

/// The patient and the bed of an admission, from an R5 `Encounter`. A bed is a `Location` here.
pub(super) fn admission_from(body: &Value) -> Result<(Uuid, Uuid), Error> {
    let status = body.get("status").and_then(Value::as_str);
    if status != Some("in-progress") {
        return Err(Error::validation(format!(
            "Encounter.status must be in-progress for an admission, got {status:?}"
        )));
    }
    let locations = body
        .get("location")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if locations != 1 {
        return Err(Error::validation(format!(
            "Encounter.location must name exactly one bed, got {locations}"
        )));
    }
    Ok((
        reference_id(body, "/subject/reference", "Patient", "Encounter.subject")?,
        reference_id(
            body,
            "/location/0/location/reference",
            "Location",
            "Encounter.location",
        )?,
    ))
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
pub async fn create_schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let saved = async {
        let schedule = schedule_from(&body)?;
        let txn = state.db.begin_txn().await?;
        let saved = save_schedule(&txn, &schedule, &request_audit_ctx(&headers)).await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(saved)
    }
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
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> FhirResult {
    let saved = async {
        let txn = state.db.begin_txn().await?;
        let mut existing = ScheduleRepository::find_by_id(&txn, id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Schedule/{id}")))?;
        let before = audit_value(&existing)?;
        if body.get("actor").is_some() || body.get("owner").is_some() {
            existing.owner = parse_schedule_owner(&body)?;
        }
        if body.get("service_type").is_some() || body.get("serviceType").is_some() {
            existing.service_type = parse_service_type(&body);
        }
        if let Some(active) = body.get("active").and_then(serde_json::Value::as_bool) {
            existing.active = active;
        }
        existing.updated_at = jiff::Timestamp::now();
        let saved = ScheduleRepository::update(&txn, &existing).await?;
        let after = audit_value(&saved)?;
        let ctx = request_audit_ctx(&headers);
        record(&txn, "Schedule", id, Some(before), Some(after), &ctx).await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(saved)
    }
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
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    async {
        let txn = state.db.begin_txn().await?;
        let existing = ScheduleRepository::find_by_id(&txn, id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Schedule/{id}")))?;
        ScheduleRepository::delete(&txn, id).await?;
        let before = Some(audit_value(&existing)?);
        record(
            &txn,
            "Schedule",
            id,
            before,
            None,
            &request_audit_ctx(&headers),
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }
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

pub(super) fn parse_dt(body: &Value, key: &str) -> Result<jiff::Timestamp, Error> {
    let raw = body
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::validation(format!("Slot.{key} is required")))?;
    // FHIR `instant`: RFC 3339 with a mandatory offset.
    raw.parse::<jiff::Timestamp>()
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
pub async fn create_slot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> FhirResult {
    let saved = async {
        let slot = slot_from(&body)?;
        let txn = state.db.begin_txn().await?;
        let saved = save_slot(&txn, &slot, &request_audit_ctx(&headers)).await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(saved)
    }
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
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> FhirResult {
    let saved = async {
        let txn = state.db.begin_txn().await?;
        let mut existing = SlotRepository::select_for_update(&txn, id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Slot/{id}")))?;
        let before = audit_value(&existing)?;
        if body.get("start").is_some() {
            existing.start_datetime = parse_dt(&body, "start")?;
        }
        if body.get("end").is_some() {
            existing.end_datetime = parse_dt(&body, "end")?;
        }
        if (body.get("start").is_some() || body.get("end").is_some())
            && existing.end_datetime <= existing.start_datetime
        {
            return Err(Error::validation("Slot.end must be after Slot.start"));
        }
        if let Some(status) = parse_slot_status(&body)? {
            existing.status = status;
        }
        existing.updated_at = jiff::Timestamp::now();
        let saved = SlotRepository::update(&txn, &existing).await?;
        let after = audit_value(&saved)?;
        let ctx = request_audit_ctx(&headers);
        record(&txn, "Slot", id, Some(before), Some(after), &ctx).await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(saved)
    }
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
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    async {
        let txn = state.db.begin_txn().await?;
        let existing = SlotRepository::select_for_update(&txn, id)
            .await?
            .ok_or_else(|| Error::not_found(format!("Slot/{id}")))?;
        SlotRepository::delete(&txn, id).await?;
        let before = Some(audit_value(&existing)?);
        record(&txn, "Slot", id, before, None, &request_audit_ctx(&headers)).await?;
        txn.commit().await.map_err(Error::Database)
    }
    .await
    .map_err(error_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    use serde_json::json;

    fn id() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn a_schedule_owner_comes_from_an_actor_reference_or_a_native_owner() {
        let u = id();
        for (kind, want) in [
            ("Practitioner", "practitioner"),
            ("Location", "bed"),
            ("Bed", "bed"),
            ("Room", "room"),
            ("Device", "equipment"),
            ("Equipment", "equipment"),
        ] {
            let body = json!({ "actor": [{ "reference": format!("{kind}/{u}") }] });
            let owner = parse_schedule_owner(&body).unwrap();
            let got = match owner {
                ScheduleOwner::Practitioner(_) => "practitioner",
                ScheduleOwner::Bed(_) => "bed",
                ScheduleOwner::Room(_) => "room",
                ScheduleOwner::Equipment(_) => "equipment",
            };
            assert_eq!(got, want, "{kind}");
        }
        for kind in ["Practitioner", "Bed", "Room"] {
            let body = json!({ "owner": { "kind": kind, "id": u.to_string() } });
            assert!(parse_schedule_owner(&body).is_ok(), "{kind}");
        }
        for bad in [
            json!({}),
            json!({ "actor": [] }),
            json!({ "actor": [{}] }),
            json!({ "actor": [{ "reference": "no-slash" }] }),
            json!({ "actor": [{ "reference": "Practitioner/not-a-uuid" }] }),
            json!({ "actor": [{ "reference": format!("Patient/{u}") }] }),
            json!({ "owner": { "kind": "Patient", "id": u.to_string() } }),
            json!({ "owner": { "kind": "Bed", "id": "nope" } }),
        ] {
            assert!(parse_schedule_owner(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_service_type_is_native_fhir_or_general() {
        assert_eq!(
            parse_service_type(&json!({ "service_type": "cardiology" })),
            "cardiology"
        );
        let fhir =
            json!({ "serviceType": [{ "concept": { "coding": [{ "code": "394579002" }] } }] });
        assert_eq!(parse_service_type(&fhir), "394579002");
        assert_eq!(parse_service_type(&json!({})), "general");
        assert_eq!(parse_service_type(&json!({ "serviceType": [] })), "general");
        assert_eq!(
            parse_service_type(&json!({ "serviceType": [{ "concept": {} }] })),
            "general"
        );
    }

    #[test]
    fn a_slot_needs_a_schedule_two_times_in_order_and_a_known_status() {
        let sched = id();
        let ok = json!({
            "schedule": { "reference": format!("Schedule/{sched}") },
            "start": "2027-01-01T09:00:00Z", "end": "2027-01-01T09:30:00Z", "status": "busy-unavailable",
        });
        let slot = slot_from(&ok).unwrap();
        assert_eq!(slot.schedule_id, sched);
        assert_eq!(slot.status, SlotStatus::BlockedOut);
        let native = json!({
            "schedule_id": sched.to_string(),
            "start": "2027-01-01T09:00:00Z", "end": "2027-01-01T09:30:00Z",
        });
        assert!(slot_from(&native).is_ok());
        let mut backwards = ok.clone();
        backwards["end"] = json!("2027-01-01T08:00:00Z");
        assert!(slot_from(&backwards).is_err());
        let mut no_start = ok.clone();
        no_start.as_object_mut().unwrap().remove("start");
        assert!(slot_from(&no_start).is_err());
        let mut bad_time = ok.clone();
        bad_time["start"] = json!("tomorrow");
        assert!(slot_from(&bad_time).is_err());
        let mut bad_status = ok.clone();
        bad_status["status"] = json!("maybe");
        assert!(slot_from(&bad_status).is_err());
        for (s, want) in [
            ("FREE", SlotStatus::Free),
            ("busy", SlotStatus::Busy),
            ("blocked_out", SlotStatus::BlockedOut),
        ] {
            assert_eq!(
                parse_slot_status(&json!({ "status": s })).unwrap(),
                Some(want),
                "{s}"
            );
        }
        assert_eq!(parse_slot_status(&json!({})).unwrap(), None);
        for bad in [
            json!({}),
            json!({ "schedule": {} }),
            json!({ "schedule": { "reference": "Slot/x" } }),
            json!({ "schedule": { "reference": "Schedule/not-a-uuid" } }),
            json!({ "schedule_id": "not-a-uuid" }),
        ] {
            assert!(parse_schedule_ref(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_booking_and_an_admission_name_exactly_one_slot_or_bed_and_a_patient() {
        let (slot, patient, bed) = (id(), id(), id());
        let booking = json!({
            "status": "booked",
            "slot": [{ "reference": format!("Slot/{slot}") }],
            "subject": { "reference": format!("Patient/{patient}") },
        });
        assert_eq!(booking_from(&booking).unwrap(), (slot, patient));
        for bad in [
            json!({ "status": "proposed" }),
            json!({ "status": "booked", "slot": [] }),
            json!({ "status": "booked", "slot": [{ "reference": format!("Slot/{slot}") }] }),
            json!({ "status": "booked", "slot": [{ "reference": "Patient/x" }], "subject": { "reference": format!("Patient/{patient}") } }),
        ] {
            assert!(booking_from(&bad).is_err(), "{bad}");
        }
        let admission = json!({
            "status": "in-progress",
            "subject": { "reference": format!("Patient/{patient}") },
            "location": [{ "location": { "reference": format!("Location/{bed}") } }],
        });
        assert_eq!(admission_from(&admission).unwrap(), (patient, bed));
        for bad in [
            json!({ "status": "planned" }),
            json!({ "status": "in-progress" }),
            json!({ "status": "in-progress", "location": [] }),
        ] {
            assert!(admission_from(&bad).is_err(), "{bad}");
        }
        assert!(is_inpatient(
            &json!({ "class": [{ "coding": [{ "code": "IMP" }] }] })
        ));
        assert!(!is_inpatient(
            &json!({ "class": [{ "coding": [{ "code": "AMB" }] }] })
        ));
        assert!(!is_inpatient(&json!({})));
    }

    #[test]
    fn a_coverage_needs_a_patient_status_kind_and_a_payer_unless_self_pay() {
        let patient = id();
        let base = json!({
            "beneficiary": { "reference": format!("Patient/{patient}") },
            "status": "active", "kind": "insurance",
            "insurer": { "display": "Payer" },
            "identifier": [{ "value": "POL-1" }],
            "period": { "start": "2026-01-01", "end": "2026-12-31T00:00:00Z" },
        });
        let c = coverage_from(&base).unwrap();
        assert_eq!((c.person_id, c.policy_number.as_str()), (patient, "POL-1"));
        for (status, want) in [
            ("cancelled", CoverageStatus::Cancelled),
            ("draft", CoverageStatus::Draft),
            ("entered-in-error", CoverageStatus::EnteredInError),
        ] {
            let mut b = base.clone();
            b["status"] = json!(status);
            assert_eq!(coverage_from(&b).unwrap().status, want);
        }
        let mut self_pay = base.clone();
        self_pay["kind"] = json!("self-pay");
        self_pay.as_object_mut().unwrap().remove("insurer");
        assert_eq!(coverage_from(&self_pay).unwrap().payor_name, "Self-pay");
        let mut other = base.clone();
        other["kind"] = json!("other");
        assert!(coverage_from(&other).is_ok());
        let mut no_payer = base.clone();
        no_payer.as_object_mut().unwrap().remove("insurer");
        assert!(coverage_from(&no_payer).is_err());
        let mut bad_kind = base.clone();
        bad_kind["kind"] = json!("barter");
        assert!(coverage_from(&bad_kind).is_err());
        let mut bad_status = base.clone();
        bad_status["status"] = json!("lapsed");
        assert!(coverage_from(&bad_status).is_err());
        let mut ends_first = base.clone();
        ends_first["period"] = json!({ "start": "2026-06-01", "end": "2026-01-01" });
        assert!(coverage_from(&ends_first).is_err());
        let mut bad_date = base.clone();
        bad_date["period"] = json!({ "start": "soon" });
        assert!(coverage_from(&bad_date).is_err());
        assert!(coverage_from(&json!({})).is_err());
    }
}

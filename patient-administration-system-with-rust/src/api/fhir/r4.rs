//! FHIR R4 reads for the Wales exchange (NEW-1): `Appointment`, `Encounter` and `ServiceRequest` (a referral).
//!
//! Wales exchanges R4 (FHIR Standards Wales, `fhir.r4.wales`, built on UK Core STU2) and the PAS FHIR endpoint
//! speaks R5, so these live under `/fhir/r4/` as the IHE Scheduling operations do (NEW-10), answer with
//! `application/fhir+json; fhirVersion=4.0`, and are mapped here from the same stored rows. R5 stays inside.
//! Each resource names its Wales profile in `meta.profile`; `scripts/validate-fhir.sh` checks them with the
//! HL7 validator against that profile and CI runs it.
//!
//! **Not built:** the waiting-list entry (the Wales IG has no profile for one, and R4 has no resource for it);
//! the clinic-code extension (`Extension-DataStandardsWales-ClinicCode`; the PAS holds no clinic code, only
//! a schedule's service type); searches and writes; `Appointment.basedOn` (the appointment model does not
//! carry its referral).

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::models::appointment::{Appointment, AppointmentStatus};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::referrals::Referral;

const CONTENT_TYPE: &str = "application/fhir+json; fhirVersion=4.0";
const WALES: &str = "https://fhir.nhs.wales/StructureDefinition/DataStandardsWales-";
const ACT_CODE: &str = "http://terminology.hl7.org/CodeSystem/v3-ActCode";
/// Where a referral's own reference number is namespaced. It is the PAS's, not a national system.
pub const REFERENCE_SYSTEM: &str = "urn:patient-administration-system:referral-reference";

fn respond(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, CONTENT_TYPE)],
        body.to_string(),
    )
        .into_response()
}

fn not_found(what: &str, id: Uuid) -> Response {
    respond(
        StatusCode::NOT_FOUND,
        &json!({
            "resourceType": "OperationOutcome",
            "issue": [{ "severity": "error", "code": "not-found", "diagnostics": format!("{what}/{id} not found") }],
        }),
    )
}

fn failed(e: &crate::Error) -> Response {
    tracing::error!(error = %e, "R4 read failed");
    respond(
        StatusCode::INTERNAL_SERVER_ERROR,
        &json!({
            "resourceType": "OperationOutcome",
            "issue": [{ "severity": "fatal", "code": "exception", "diagnostics": "the resource could not be read" }],
        }),
    )
}

fn appointment_status(s: AppointmentStatus) -> &'static str {
    match s {
        AppointmentStatus::Proposed => "proposed",
        AppointmentStatus::Booked => "booked",
        AppointmentStatus::Arrived => "arrived",
        AppointmentStatus::Fulfilled => "fulfilled",
        AppointmentStatus::Cancelled => "cancelled",
        AppointmentStatus::NoShow => "noshow",
    }
}

/// An appointment as a Wales R4 `Appointment`.
pub fn appointment(a: &Appointment) -> Value {
    let mut participants = vec![json!({
        "actor": { "reference": format!("Patient/{}", a.person_id) },
        "status": "accepted",
    })];
    if let Some(w) = a.worker_id {
        participants.push(json!({
            "actor": { "reference": format!("Practitioner/{w}") },
            "status": "accepted",
        }));
    }
    let mut r = json!({
        "resourceType": "Appointment",
        "id": a.id,
        "meta": { "profile": [format!("{WALES}Appointment")] },
        "status": appointment_status(a.status),
        "start": a.start_datetime,
        "end": a.end_datetime,
        "created": a.created_at,
        "participant": participants,
    });
    if let Some(slot) = a.slot_id {
        r["slot"] = json!([{ "reference": format!("Slot/{slot}") }]);
    }
    if let Some(reason) = a.reason.as_deref().filter(|t| !t.trim().is_empty()) {
        r["description"] = json!(reason);
    }
    if a.status == AppointmentStatus::Cancelled
        && let Some(why) = a
            .cancellation_reason
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok())
            .and_then(|v| v.as_str().map(str::to_string))
    {
        r["cancelationReason"] = json!({ "text": why });
    }
    r
}

fn encounter_status(s: EncounterStatus) -> &'static str {
    match s {
        EncounterStatus::Planned => "planned",
        EncounterStatus::Arrived => "arrived",
        EncounterStatus::InProgress => "in-progress",
        EncounterStatus::OnLeave => "onleave",
        EncounterStatus::Finished => "finished",
        EncounterStatus::Cancelled => "cancelled",
    }
}

/// The `v3-ActCode` of an encounter class: ambulatory, inpatient, emergency, short stay, home health, virtual.
fn encounter_class(c: EncounterClass) -> (&'static str, &'static str) {
    match c {
        EncounterClass::Outpatient => ("AMB", "ambulatory"),
        EncounterClass::Inpatient => ("IMP", "inpatient encounter"),
        EncounterClass::Emergency => ("EMER", "emergency"),
        EncounterClass::DayCase => ("SS", "short stay"),
        EncounterClass::HomeCare => ("HH", "home health"),
        EncounterClass::Virtual => ("VR", "virtual"),
    }
}

/// An encounter as a Wales R4 `Encounter`.
pub fn encounter(e: &Encounter) -> Value {
    let (code, display) = encounter_class(e.class);
    let mut period = json!({ "start": e.period_start });
    if let Some(end) = e.period_end {
        period["end"] = json!(end);
    }
    let mut r = json!({
        "resourceType": "Encounter",
        "id": e.id,
        "meta": { "profile": [format!("{WALES}Encounter")] },
        "status": encounter_status(e.status),
        "class": { "system": ACT_CODE, "code": code, "display": display },
        "subject": { "reference": format!("Patient/{}", e.person_id) },
        "period": period,
    });
    if let Some(w) = e.worker_id {
        r["participant"] = json!([{ "individual": { "reference": format!("Practitioner/{w}") } }]);
    }
    if let Some(reason) = e.reason.as_deref().filter(|t| !t.trim().is_empty()) {
        r["reasonCode"] = json!([{ "text": reason }]);
    }
    r
}

fn request_status(referral_status: &str) -> &'static str {
    match referral_status {
        "closed" | "returned_with_advice" => "completed",
        "redirected" => "revoked",
        _ => "active",
    }
}

/// A referral as a Wales R4 `ServiceRequest`.
pub fn service_request(r: &Referral) -> Value {
    let mut out = json!({
        "resourceType": "ServiceRequest",
        "id": r.id,
        "meta": { "profile": [format!("{WALES}ServiceRequest")] },
        "status": request_status(&r.status),
        "intent": "order",
        "subject": { "reference": format!("Patient/{}", r.person_id) },
        "authoredOn": r.received_at,
        "code": { "text": r.target_service },
    });
    if let Some(reference) = r.external_ref.as_deref().filter(|t| !t.trim().is_empty()) {
        out["identifier"] = json!([{ "system": REFERENCE_SYSTEM, "value": reference }]);
    }
    if let Some(w) = r.referring_worker_id {
        out["requester"] = json!({ "reference": format!("Practitioner/{w}") });
    }
    if let Some(reason) = r.reason.as_deref().filter(|t| !t.trim().is_empty()) {
        out["reasonCode"] = json!([{ "text": reason }]);
    }
    out
}

macro_rules! read_handler {
    ($name:ident, $path:literal, $summary:literal, $kind:literal, |$state:ident, $id:ident| $load:expr, $map:expr) => {
        #[utoipa::path(
            get,
            path = $path,
            tag = "FHIR",
            summary = $summary,
            params(("id" = String, Path, description = "Path parameter `id`")),
            responses(
                (status = 200, description = "FHIR R4 resource (`application/fhir+json; fhirVersion=4.0`)"),
                (status = "4XX", description = "Error; an OperationOutcome"),
                (status = "5XX", description = "Server error; an OperationOutcome"),
            )
        )]
        pub async fn $name(
            State($state): State<AppState>,
            Path($id): Path<Uuid>,
            _headers: HeaderMap,
        ) -> Response {
            match $load.await {
                Ok(Some(found)) => respond(StatusCode::OK, &$map(&found)),
                Ok(None) => not_found($kind, $id),
                Err(e) => failed(&e),
            }
        }
    };
}

read_handler!(
    get_appointment,
    "/fhir/r4/Appointment/{id}",
    "Get an appointment (FHIR R4, DataStandardsWales-Appointment)",
    "Appointment",
    |state, id| AppointmentRepository::find_by_id(&state.db, id),
    appointment
);
read_handler!(
    get_encounter,
    "/fhir/r4/Encounter/{id}",
    "Get an encounter (FHIR R4, DataStandardsWales-Encounter)",
    "Encounter",
    |state, id| EncounterRepository::find_by_id(&state.db, id),
    encounter
);
read_handler!(
    get_service_request,
    "/fhir/r4/ServiceRequest/{id}",
    "Get a referral as a ServiceRequest (FHIR R4, DataStandardsWales-ServiceRequest)",
    "ServiceRequest",
    |state, id| referral(&state, id),
    service_request
);

async fn referral(state: &AppState, id: Uuid) -> crate::Result<Option<Referral>> {
    match state.referrals.get(id).await {
        Ok(r) => Ok(Some(r)),
        Err(crate::Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn an_appointment_maps_status_participants_and_slot() {
        let mut a = Appointment::new(Uuid::new_v4(), Utc::now(), Utc::now());
        a.status = AppointmentStatus::NoShow;
        a.slot_id = Some(Uuid::new_v4());
        a.worker_id = Some(Uuid::new_v4());
        let r = appointment(&a);
        assert_eq!(r["status"], "noshow");
        assert_eq!(r["participant"].as_array().unwrap().len(), 2);
        assert_eq!(r["participant"][0]["status"], "accepted");
        assert!(
            r["slot"][0]["reference"]
                .as_str()
                .unwrap()
                .starts_with("Slot/")
        );
        assert!(
            r["meta"]["profile"][0]
                .as_str()
                .unwrap()
                .ends_with("DataStandardsWales-Appointment")
        );
    }

    #[test]
    fn an_encounter_class_is_a_v3_act_code_and_status_is_hyphenated() {
        let mut e = Encounter::new(Uuid::new_v4(), EncounterClass::DayCase);
        e.status = EncounterStatus::InProgress;
        let r = encounter(&e);
        assert_eq!(r["status"], "in-progress");
        assert_eq!(r["class"]["code"], "SS");
        assert_eq!(r["class"]["system"], ACT_CODE);
    }

    #[test]
    fn a_referral_ends_as_completed_or_revoked_and_is_otherwise_active() {
        for (s, want) in [
            ("received", "active"),
            ("waitlisted", "active"),
            ("booked", "active"),
            ("closed", "completed"),
            ("returned_with_advice", "completed"),
            ("redirected", "revoked"),
        ] {
            assert_eq!(request_status(s), want, "{s}");
        }
    }
}

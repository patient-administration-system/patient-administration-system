//! FHIR R5 mappings for the PAS-owned reads (QUAL-39): `Appointment`, `Encounter`, `Slot` and `Schedule`.
//!
//! These replace the stored row with a `resourceType` added. R5 renamed or reshaped several elements the R4
//! mappings in [`super::r4`] use: `Appointment.reasonCode` is `reason` (a `CodeableReference`),
//! `Encounter.class` is a list, `Encounter.period` is `actualPeriod`, `Encounter.reasonCode` is
//! `reason.value`, `Schedule.serviceType` is a list of `CodeableReference`, and the encounter status
//! codes changed (`finished` is `completed`; `arrived` and `onleave` are a `subjectStatus` on a `planned`
//! or `on-hold` encounter). `scripts/validate-fhir.sh` checks each against the HL7 validator.
//!
//! **Not built:** `Slot.serviceType`, `Appointment.serviceType`, and `Appointment.basedOn`; the models do not
//! carry a coded service for those.

use serde_json::{Value, json};

use crate::models::appointment::{Appointment, AppointmentStatus};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::schedule::{Schedule, ScheduleOwner, Slot, SlotStatus};

const ACT_CODE: &str = "http://terminology.hl7.org/CodeSystem/v3-ActCode";
const SUBJECT_STATUS: &str = "http://terminology.hl7.org/CodeSystem/encounter-subject-status";

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

/// An appointment as an R5 `Appointment`.
pub fn appointment(a: &Appointment) -> Value {
    let patient = json!({ "reference": format!("Patient/{}", a.person_id) });
    let mut participants = vec![json!({ "actor": patient, "status": "accepted" })];
    if let Some(w) = a.worker_id {
        participants.push(json!({
            "actor": { "reference": format!("Practitioner/{w}") },
            "status": "accepted",
        }));
    }
    let mut r = json!({
        "resourceType": "Appointment",
        "id": a.id,
        "status": appointment_status(a.status),
        "subject": patient,
        "start": a.start_datetime,
        "end": a.end_datetime,
        "created": a.created_at,
        "participant": participants,
    });
    if let Some(slot) = a.slot_id {
        r["slot"] = json!([{ "reference": format!("Slot/{slot}") }]);
    }
    if let Some(reason) = a.reason.as_deref().filter(|t| !t.trim().is_empty()) {
        r["reason"] = json!([{ "concept": { "text": reason } }]);
    }
    if a.status == AppointmentStatus::Cancelled
        && let Some(why) = a
            .cancellation_reason
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok())
            .and_then(|v| v.as_str().map(str::to_string))
    {
        r["cancellationReason"] = json!({ "text": why });
    }
    r
}

/// The R5 status, and the `subjectStatus` code when R5 keeps the PAS state there instead.
fn encounter_status(s: EncounterStatus) -> (&'static str, Option<&'static str>) {
    match s {
        EncounterStatus::Planned => ("planned", None),
        EncounterStatus::Arrived => ("planned", Some("arrived")),
        EncounterStatus::InProgress => ("in-progress", None),
        EncounterStatus::OnLeave => ("on-hold", Some("on-leave")),
        EncounterStatus::Finished => ("completed", None),
        EncounterStatus::Cancelled => ("cancelled", None),
    }
}

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

/// An encounter as an R5 `Encounter`.
pub fn encounter(e: &Encounter) -> Value {
    let (code, display) = encounter_class(e.class);
    let (status, subject_status) = encounter_status(e.status);
    let mut period = json!({ "start": e.period_start });
    if let Some(end) = e.period_end {
        period["end"] = json!(end);
    }
    let mut r = json!({
        "resourceType": "Encounter",
        "id": e.id,
        "status": status,
        "class": [{ "coding": [{ "system": ACT_CODE, "code": code, "display": display }] }],
        "subject": { "reference": format!("Patient/{}", e.person_id) },
        "actualPeriod": period,
    });
    if let Some(code) = subject_status {
        r["subjectStatus"] = json!({ "coding": [{ "system": SUBJECT_STATUS, "code": code }] });
    }
    if let Some(w) = e.worker_id {
        r["participant"] = json!([{ "actor": { "reference": format!("Practitioner/{w}") } }]);
    }
    if let Some(reason) = e.reason.as_deref().filter(|t| !t.trim().is_empty()) {
        r["reason"] = json!([{ "value": [{ "concept": { "text": reason } }] }]);
    }
    r
}

/// A slot as an R5 `Slot`.
pub fn slot(s: &Slot) -> Value {
    json!({
        "resourceType": "Slot",
        "id": s.id,
        "schedule": { "reference": format!("Schedule/{}", s.schedule_id) },
        "status": match s.status {
            SlotStatus::Free => "free",
            SlotStatus::Busy => "busy",
            SlotStatus::BlockedOut => "busy-unavailable",
        },
        "start": s.start_datetime,
        "end": s.end_datetime,
    })
}

/// A schedule as an R5 `Schedule`. Its owner is the one `actor`: a practitioner, a location (a bed or a
/// room) or a device (equipment).
pub fn schedule(s: &Schedule) -> Value {
    let actor = match &s.owner {
        ScheduleOwner::Practitioner(id) => format!("Practitioner/{id}"),
        ScheduleOwner::Bed(id) | ScheduleOwner::Room(id) => format!("Location/{id}"),
        ScheduleOwner::Equipment(id) => format!("Device/{id}"),
    };
    let mut r = json!({
        "resourceType": "Schedule",
        "id": s.id,
        "active": s.active,
        "actor": [{ "reference": actor }],
    });
    if !s.service_type.trim().is_empty() {
        r["serviceType"] = json!([{ "concept": { "text": s.service_type } }]);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn slot_names_its_schedule_and_blocked_out_is_busy_unavailable() {
        let mut s = Slot::new(Uuid::new_v4(), Utc::now(), Utc::now());
        s.status = SlotStatus::BlockedOut;
        let v = slot(&s);
        assert_eq!(v["status"], "busy-unavailable");
        assert_eq!(
            v["schedule"]["reference"],
            format!("Schedule/{}", s.schedule_id)
        );
        assert!(v.get("start_datetime").is_none());
    }

    #[test]
    fn schedule_has_an_actor_per_owner_kind() {
        let id = Uuid::new_v4();
        let v = schedule(&Schedule::new(ScheduleOwner::Bed(id), "general".into()));
        assert_eq!(v["actor"][0]["reference"], format!("Location/{id}"));
        assert_eq!(v["serviceType"][0]["concept"]["text"], "general");
        let v = schedule(&Schedule::new(ScheduleOwner::Equipment(id), String::new()));
        assert_eq!(v["actor"][0]["reference"], format!("Device/{id}"));
        assert!(v.get("serviceType").is_none());
    }

    #[test]
    fn arrived_encounter_is_planned_with_a_subject_status() {
        assert_eq!(
            encounter_status(EncounterStatus::Arrived),
            ("planned", Some("arrived"))
        );
        assert_eq!(
            encounter_status(EncounterStatus::Finished),
            ("completed", None)
        );
    }
}

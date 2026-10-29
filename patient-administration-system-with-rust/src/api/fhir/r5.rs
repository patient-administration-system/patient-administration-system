//! FHIR R5 mappings for the PAS-owned reads (QUAL-39): `Appointment`, `Encounter`, `Slot` and `Schedule`.
//!
//! These replace the stored row with a `resourceType` added. R5 renamed or reshaped several elements the R4
//! mappings in [`super::r4`] use: `Appointment.reasonCode` is `reason` (a `CodeableReference`),
//! `Encounter.class` is a list, `Encounter.period` is `actualPeriod`, `Encounter.reasonCode` is
//! `reason.value`, `Schedule.serviceType` is a list of `CodeableReference`, and the encounter status
//! codes changed (`finished` is `completed`; `arrived` and `onleave` are a `subjectStatus` on a `planned`
//! or `on-hold` encounter). `scripts/validate-fhir.sh` checks each against the HL7 validator.
//!
//! Location (a bed), Coverage, Patient and Practitioner follow, and so do the `searchset` Bundles of Flag and
//! Observation. Patient and Practitioner are built from the upstream's open `data` map, so the mapping reads
//! what is there and assumes none of it: a name need not be given-and-family (a mononym or a `text` name is
//! kept), an identifier carries its own system, an address has no national shape, and a telecom or gender value
//! outside the R5 code set becomes `other` or `unknown` rather than being dropped or invented (NFR-26).
//!
//! **Not built:** `Slot.serviceType`, `Appointment.serviceType`, and `Appointment.basedOn`; the models do not
//! carry a coded service for those.

use axum::http::{HeaderMap, header};
use serde_json::{Map, Value, json};

use crate::models::appointment::{Appointment, AppointmentStatus};
use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::{Bed, BedStatus};
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

/// The address this R5 endpoint answers at: `PAS_FHIR_R5_BASE_URL` (no trailing slash) when set, otherwise built
/// from the request's `Host` (and `X-Forwarded-Proto`). A `searchset` entry's `fullUrl` and the relative
/// references inside it are resolved against it.
pub fn base_url(headers: &HeaderMap) -> String {
    if let Some(b) = crate::config::env::text("PAS_FHIR_R5_BASE_URL") {
        return b.trim_end_matches('/').to_string();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| !h.trim().is_empty())
        .unwrap_or("localhost");
    let scheme = match headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
    {
        Some("https") => "https",
        _ => "http",
    };
    format!("{scheme}://{host}/fhir")
}

/// A `searchset` Bundle: each entry has the `fullUrl` and `search.mode` R5 asks for, and the Bundle a `self`
/// link naming the search (`self_url` is the path and query that was asked).
pub fn searchset(base: &str, self_url: &str, resources: Vec<Value>) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": resources.len(),
        "link": [{ "relation": "self", "url": format!("{base}{self_url}") }],
        "entry": resources.into_iter().map(|r| json!({
            "fullUrl": format!("{base}/{}/{}", r["resourceType"].as_str().unwrap_or_default(), r["id"].as_str().unwrap_or_default()),
            "resource": r,
            "search": { "mode": "match" },
        })).collect::<Vec<_>>(),
    })
}

/// A bed as an R5 `Location` (`form` bed, `operationalStatus` from the HL7 v2 bed status table).
pub fn location(b: &Bed) -> Value {
    let (status, op) = match b.status {
        BedStatus::Available => ("active", "U"),
        BedStatus::Occupied => ("active", "O"),
        BedStatus::Reserved => ("active", "U"),
        BedStatus::OutOfService => ("inactive", "C"),
        BedStatus::Cleaning => ("active", "H"),
    };
    json!({
        "resourceType": "Location",
        "id": b.id,
        "identifier": [{ "value": b.code }],
        "status": status,
        "operationalStatus": { "system": "http://terminology.hl7.org/CodeSystem/v2-0116", "code": op },
        "name": b.name,
        "mode": "instance",
        "form": { "coding": [{
            "system": "http://terminology.hl7.org/CodeSystem/location-physical-type",
            "code": "bd",
            "display": "Bed",
        }] },
    })
}

const RELATIONSHIPS: [&str; 7] = [
    "child", "parent", "spouse", "common", "other", "self", "injured",
];

/// A coverage as an R5 `Coverage`. The payer is the `insurer` by name and identifier; the policy number is the
/// member-number `identifier`; a group number is a `class` of type `group`.
pub fn coverage(c: &Coverage) -> Value {
    let mut insurer = json!({ "display": c.payor_name });
    if let Some(id) = c
        .payor_identifier
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        insurer["identifier"] = json!({ "value": id });
    }
    let mut period = json!({ "start": c.start_date });
    if let Some(end) = c.end_date {
        period["end"] = json!(end);
    }
    let mut r = json!({
        "resourceType": "Coverage",
        "id": c.id,
        "status": match c.status {
            CoverageStatus::Active => "active",
            CoverageStatus::Cancelled => "cancelled",
            CoverageStatus::Draft => "draft",
            CoverageStatus::EnteredInError => "entered-in-error",
        },
        "kind": match c.kind {
            CoverageKind::Insurance => "insurance",
            CoverageKind::SelfPay => "self-pay",
            CoverageKind::Other => "other",
        },
        "beneficiary": { "reference": format!("Patient/{}", c.person_id) },
        "insurer": insurer,
        "period": period,
    });
    if !c.policy_number.trim().is_empty() {
        r["identifier"] = json!([{
            "type": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/v2-0203", "code": "MB" }] },
            "value": c.policy_number,
        }]);
    }
    if let Some(sub) = c.subscriber_id {
        r["subscriber"] = json!({ "reference": format!("Patient/{sub}") });
    }
    if let Some(g) = c.group_number.as_deref().filter(|t| !t.trim().is_empty()) {
        r["class"] = json!([{
            "type": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/coverage-class", "code": "group" }] },
            "value": { "value": g },
        }]);
    }
    let rel = c.relationship.trim().to_lowercase();
    if !rel.is_empty() {
        r["relationship"] = if RELATIONSHIPS.contains(&rel.as_str()) {
            json!({ "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/subscriber-relationship", "code": rel }] })
        } else {
            json!({ "text": c.relationship })
        };
    }
    r
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn coded(v: &Value, key: &str, allowed: &[&str], other: &str) -> Option<Value> {
    text(v, key).map(|t| {
        let t = t.to_lowercase();
        json!(if allowed.contains(&t.as_str()) {
            t.as_str()
        } else {
            other
        })
    })
}

/// A name from the upstream: an object (`family`, `given` as a list or one string, `text`, `prefix`, `suffix`)
/// or a bare string, which is kept as `text`. Nothing is split or reordered.
fn human_name(v: &Value) -> Option<Value> {
    let mut out = Map::new();
    match v {
        Value::String(t) if !t.trim().is_empty() => {
            out.insert("text".into(), json!(t.trim()));
        }
        Value::Object(_) => {
            for (src, dst) in [("family", "family"), ("text", "text")] {
                if let Some(t) = text(v, src) {
                    out.insert(dst.into(), json!(t));
                }
            }
            for key in ["given", "prefix", "suffix"] {
                let list: Vec<&str> = match v.get(key) {
                    Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
                    Some(Value::String(t)) => vec![t.as_str()],
                    _ => vec![],
                };
                let list: Vec<&str> = list
                    .into_iter()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .collect();
                if !list.is_empty() {
                    out.insert(key.into(), json!(list));
                }
            }
        }
        _ => {}
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

fn list<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    match v.get(key) {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(o @ Value::Object(_)) => vec![o],
        _ => vec![],
    }
}

/// A person or worker from the upstream as an R5 `Patient` or `Practitioner`. Reads `name`, `gender`,
/// `birth_date`, `identifiers`, `telecom`, `addresses` and `active`; whatever else the upstream sends is not
/// carried. A `birth_date` that is not a FHIR date (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`) is left out.
pub fn person(resource_type: &str, id: uuid::Uuid, data: &Map<String, Value>) -> Value {
    let v = Value::Object(data.clone());
    let mut r = json!({ "resourceType": resource_type, "id": id });
    if let Some(a) = v.get("active").and_then(Value::as_bool) {
        r["active"] = json!(a);
    }
    let names: Vec<Value> = list(&v, "name")
        .into_iter()
        .filter_map(human_name)
        .collect();
    if !names.is_empty() {
        r["name"] = json!(names);
    }
    if let Some(g) = coded(
        &v,
        "gender",
        &["male", "female", "other", "unknown"],
        "other",
    ) {
        r["gender"] = g;
    }
    if let Some(d) = text(&v, "birth_date").filter(|d| {
        let parts: Vec<&str> = d.split('-').collect();
        (1..=3).contains(&parts.len())
            && parts.iter().enumerate().all(|(i, p)| {
                p.len() == if i == 0 { 4 } else { 2 } && p.bytes().all(|b| b.is_ascii_digit())
            })
    }) {
        r["birthDate"] = json!(d);
    }
    let identifiers: Vec<Value> = list(&v, "identifiers")
        .into_iter()
        .filter_map(|i| {
            let value = text(i, "value")?;
            let mut o = json!({ "value": value });
            // An identifier's system is a URI; anything else the upstream calls it is its type, as text.
            if let Some(sys) = text(i, "system").filter(|s| s.contains(':')) {
                o["system"] = json!(sys);
            } else if let Some(t) = text(i, "system").or_else(|| text(i, "type")) {
                o["type"] = json!({ "text": t });
            }
            Some(o)
        })
        .collect();
    if !identifiers.is_empty() {
        r["identifier"] = json!(identifiers);
    }
    let telecom: Vec<Value> = list(&v, "telecom")
        .into_iter()
        .filter_map(|t| {
            let value = text(t, "value")?;
            let mut o = json!({ "value": value });
            if let Some(s) = coded(
                t,
                "system",
                &["phone", "fax", "email", "pager", "url", "sms", "other"],
                "other",
            ) {
                o["system"] = s;
            }
            if let Some(u) =
                coded(t, "use", &["home", "work", "temp", "old", "mobile"], "").filter(|u| u != "")
            {
                o["use"] = u;
            }
            Some(o)
        })
        .collect();
    if !telecom.is_empty() {
        r["telecom"] = json!(telecom);
    }
    let addresses: Vec<Value> = list(&v, "addresses")
        .into_iter()
        .filter_map(|a| {
            let mut o = Map::new();
            let lines: Vec<&str> = ["line1", "line2"]
                .iter()
                .filter_map(|k| text(a, k))
                .collect();
            if !lines.is_empty() {
                o.insert("line".into(), json!(lines));
            }
            for (src, dst) in [
                ("city", "city"),
                ("state", "state"),
                ("postal_code", "postalCode"),
                ("country", "country"),
            ] {
                if let Some(t) = text(a, src) {
                    o.insert(dst.into(), json!(t));
                }
            }
            if let Some(u) = coded(
                a,
                "use_type",
                &["home", "work", "temp", "old", "billing"],
                "",
            )
            .filter(|u| u != "")
            {
                o.insert("use".into(), u);
            }
            (!o.is_empty()).then_some(Value::Object(o))
        })
        .collect();
    if !addresses.is_empty() {
        r["address"] = json!(addresses);
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
    #[test]
    fn a_person_keeps_a_mononym_and_a_non_uri_identifier_system_and_drops_a_bad_date() {
        let data = json!({
            "name": { "text": "Sukarno" },
            "gender": "nonbinary",
            "birth_date": "15/01/1990",
            "identifiers": [{ "system": "national-id", "value": "A-1" }, { "system": "urn:oid:1.2.3", "value": "B-2" }],
            "telecom": [{ "system": "whatsapp", "value": "+62 21 555 0100" }],
            "addresses": [{ "line1": "Jl. Merdeka 1", "city": "Jakarta", "postal_code": "10110", "country": "ID" }],
        });
        let v = person("Patient", Uuid::nil(), data.as_object().unwrap());
        assert_eq!(v["name"][0]["text"], "Sukarno");
        assert!(v["name"][0].get("family").is_none());
        assert_eq!(v["gender"], "other");
        assert!(v.get("birthDate").is_none());
        assert_eq!(v["identifier"][0]["type"]["text"], "national-id");
        assert_eq!(v["identifier"][1]["system"], "urn:oid:1.2.3");
        assert_eq!(v["telecom"][0]["system"], "other");
        assert_eq!(v["address"][0]["postalCode"], "10110");
    }

    #[test]
    fn searchset_has_full_urls_a_self_link_and_match_modes() {
        let id = Uuid::new_v4().to_string();
        let b = searchset(
            "https://h/fhir",
            "/Flag?patient=x",
            vec![json!({ "resourceType": "Flag", "id": id })],
        );
        assert_eq!(b["link"][0]["url"], "https://h/fhir/Flag?patient=x");
        assert_eq!(
            b["entry"][0]["fullUrl"],
            format!("https://h/fhir/Flag/{id}")
        );
        assert_eq!(b["entry"][0]["search"]["mode"], "match");
    }
}

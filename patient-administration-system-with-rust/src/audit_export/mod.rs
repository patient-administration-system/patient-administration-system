//! `audit_log` rows as FHIR R4 `AuditEvent`s in the shape of IHE Basic Audit Log Patterns (BALP) (NEW-9).
//!
//! BALP targets FHIR R4 (4.0.1), and the PAS FHIR endpoint speaks R5, so the export has its own route
//! (`GET /api/audit/balp`) and is **not** in the R5 capability statement. An event names a BALP profile in
//! `meta.profile` only when [`violations`] finds nothing wrong with it against the constraints below; a row
//! that cannot meet them (no client address, no user, an action that is not a create, read, update or
//! delete) is still exported, without the claim.
//!
//! The constraints checked are those of the BALP `Read`, `Create`, `Update` and `Delete` profiles and their
//! `Patient` variants as published at <https://profiles.ihe.net/ITI/BALP/> (read 2026-10-23): `type` is
//! `rest`; one `subtype` from the RESTful interaction codes; `action` `R`, `C`, `U` or `D`; `outcome` `0`;
//! a `recorded` instant; a client agent (DICOM 110153) and a server agent (DICOM 110152), each with `who`,
//! `requestor` and `network`; an optional user agent (`IRCP`, `requestor` true); `source.observer`; a data
//! entity (type 2) with `what` and `role`; and, for the Patient variants, a patient entity (type 1, role 1).
//! This is a structural check written from those pages, **not** the FHIR validator run against the
//! published profiles, which has not been run.

use serde_json::{Value, json};

use crate::db::entities::audit_log;

const PROFILE_BASE: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/";
const DCM: &str = "http://dicom.nema.org/resources/ontology/DCM";
const ENTITY_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-entity-type";
const ENTITY_ROLE: &str = "http://terminology.hl7.org/CodeSystem/object-role";
const INTERACTION: &str = "http://hl7.org/fhir/restful-interaction";
const AUDIT_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-event-type";
const PARTICIPATION: &str = "http://terminology.hl7.org/CodeSystem/v3-ParticipationType";
/// DICOM "Source Role ID": the client.
const CLIENT: &str = "110153";
/// DICOM "Destination Role ID": the server.
const SERVER: &str = "110152";

/// The name the PAS gives itself as the system that observed the event.
pub const SYSTEM_NAME: &str = "patient-administration-system";

/// What a row says happened, in BALP terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Create,
    Update,
    Delete,
    /// Anything else (a domain verb with no old and new value): action `E`, no BALP profile.
    Execute,
}

impl Kind {
    fn of(row: &audit_log::Model) -> Self {
        if row.action == "read" {
            return Self::Read;
        }
        match (&row.old_value, &row.new_value) {
            (None, Some(_)) => Self::Create,
            (Some(_), None) => Self::Delete,
            (Some(_), Some(_)) => Self::Update,
            (None, None) => Self::Execute,
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Read => "R",
            Self::Create => "C",
            Self::Update => "U",
            Self::Delete => "D",
            Self::Execute => "E",
        }
    }

    fn interaction(self) -> Option<&'static str> {
        match self {
            Self::Read => Some("read"),
            Self::Create => Some("create"),
            Self::Update => Some("update"),
            Self::Delete => Some("delete"),
            Self::Execute => None,
        }
    }

    fn profile_name(self) -> Option<&'static str> {
        match self {
            Self::Read => Some("Read"),
            Self::Create => Some("Create"),
            Self::Update => Some("Update"),
            Self::Delete => Some("Delete"),
            Self::Execute => None,
        }
    }
}

/// The FHIR resource type behind an `entity_type`, where there is one.
fn resource_type(entity_type: &str) -> Option<&'static str> {
    match entity_type {
        "patient" => Some("Patient"),
        "appointment" => Some("Appointment"),
        "encounter" => Some("Encounter"),
        "slot" => Some("Slot"),
        _ => None,
    }
}

fn coding(system: &str, code: &str) -> Value {
    json!({ "system": system, "code": code })
}

/// A row as an `AuditEvent`. `server_address` is this server's machine name, when known.
pub fn to_audit_event(row: &audit_log::Model, server_address: Option<&str>) -> Value {
    let kind = Kind::of(row);
    let is_patient = row.entity_type == "patient";
    let what = match resource_type(&row.entity_type) {
        Some(t) => json!({ "reference": format!("{t}/{}", row.entity_id) }),
        None => json!({ "reference": format!("urn:uuid:{}", row.entity_id) }),
    };

    let mut client = json!({
        "type": { "coding": [coding(DCM, CLIENT)] },
        "requestor": false,
    });
    if let Some(agent) = row.user_agent.as_deref().filter(|a| !a.trim().is_empty()) {
        client["who"] = json!({ "identifier": { "value": agent } });
    }
    if let Some(ip) = row.user_ip.as_deref().filter(|a| !a.trim().is_empty()) {
        client["network"] = json!({ "address": ip, "type": "2" });
    }
    let mut server = json!({
        "type": { "coding": [coding(DCM, SERVER)] },
        "who": { "identifier": { "value": SYSTEM_NAME } },
        "requestor": false,
    });
    if let Some(addr) = server_address.filter(|a| !a.trim().is_empty()) {
        server["network"] = json!({ "address": addr, "type": "1" });
    }
    let mut agents = vec![client, server];
    if let Some(user) = row.user_id.as_deref().filter(|u| !u.trim().is_empty()) {
        agents.push(json!({
            "type": { "coding": [coding(PARTICIPATION, "IRCP")] },
            "who": { "identifier": { "value": user } },
            "requestor": true,
        }));
    }

    let mut entities = vec![json!({
        "what": what,
        "type": coding(ENTITY_TYPE, "2"),
        "role": coding(ENTITY_ROLE, "4"),
    })];
    if is_patient {
        entities.push(json!({
            "what": { "reference": format!("Patient/{}", row.entity_id) },
            "type": coding(ENTITY_TYPE, "1"),
            "role": coding(ENTITY_ROLE, "1"),
        }));
    }

    let mut event = json!({
        "resourceType": "AuditEvent",
        "id": row.id,
        "type": coding(AUDIT_TYPE, "rest"),
        "action": kind.action(),
        "recorded": row.at.with_timezone(&chrono::Utc),
        "outcome": "0",
        "agent": agents,
        "source": { "observer": { "type": "Device", "identifier": { "value": SYSTEM_NAME } } },
        "entity": entities,
    });
    if let Some(i) = kind.interaction() {
        event["subtype"] = json!([coding(INTERACTION, i)]);
    }
    if let Some(name) = kind.profile_name() {
        let prefix = if is_patient { "Patient" } else { "" };
        let profile = format!("{PROFILE_BASE}IHE.BasicAudit.{prefix}{name}");
        let mut candidate = event.clone();
        candidate["meta"] = json!({ "profile": [profile.clone()] });
        if violations(&candidate).is_empty() {
            event = candidate;
        }
    }
    event
}

fn has_coding(v: &Value, system: &str, code: &str) -> bool {
    let one = |c: &Value| c["system"] == system && c["code"] == code;
    one(v) || v["coding"].as_array().is_some_and(|a| a.iter().any(one))
}

fn present(v: &Value) -> bool {
    !v.is_null()
}

/// What is wrong with an `AuditEvent` as a BALP Read, Create, Update or Delete event (and the Patient
/// variant when it names a patient entity). Empty when nothing is. See the module note on what this is.
pub fn violations(e: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut need = |ok: bool, what: &str| {
        if !ok {
            out.push(what.to_string());
        }
    };
    need(e["resourceType"] == "AuditEvent", "resourceType AuditEvent");
    need(has_coding(&e["type"], AUDIT_TYPE, "rest"), "type rest");
    let action = e["action"].as_str().unwrap_or_default();
    need(
        ["R", "C", "U", "D"].contains(&action),
        "action R, C, U or D",
    );
    let wanted = match action {
        "R" => "read",
        "C" => "create",
        "U" => "update",
        "D" => "delete",
        _ => "",
    };
    need(
        e["subtype"]
            .as_array()
            .is_some_and(|s| s.iter().any(|c| has_coding(c, INTERACTION, wanted))),
        "subtype matching the action",
    );
    need(e["outcome"] == "0", "outcome 0");
    need(e["recorded"].as_str().is_some(), "recorded");

    let agents = e["agent"].as_array().cloned().unwrap_or_default();
    let of_type = |system: &str, code: &str| -> Vec<&Value> {
        agents
            .iter()
            .filter(|a| has_coding(&a["type"], system, code))
            .collect()
    };
    for (role, code) in [("client", CLIENT), ("server", SERVER)] {
        let found = of_type(DCM, code);
        need(found.len() == 1, &format!("exactly one {role} agent"));
        if let Some(a) = found.first() {
            need(present(&a["who"]), &format!("{role} agent who"));
            need(present(&a["requestor"]), &format!("{role} agent requestor"));
            need(present(&a["network"]), &format!("{role} agent network"));
            need(a["media"].is_null(), &format!("{role} agent has no media"));
        }
    }
    let users = of_type(PARTICIPATION, "IRCP");
    need(users.len() <= 1, "at most one user agent");
    for u in users {
        need(present(&u["who"]), "user agent who");
        need(u["requestor"] == true, "user agent requestor true");
    }

    need(present(&e["source"]["observer"]), "source.observer");

    let entities = e["entity"].as_array().cloned().unwrap_or_default();
    let data: Vec<_> = entities
        .iter()
        .filter(|x| has_coding(&x["type"], ENTITY_TYPE, "2"))
        .collect();
    need(data.len() == 1, "exactly one data entity (type 2)");
    for d in data {
        need(present(&d["what"]), "data entity what");
        need(present(&d["role"]), "data entity role");
    }
    for x in &entities {
        need(
            x["name"].is_null() || x["query"].is_null(),
            "entity has not both name and query",
        );
    }
    // The Patient variants add a patient entity; an event with one must carry it complete.
    let patients: Vec<_> = entities
        .iter()
        .filter(|x| has_coding(&x["type"], ENTITY_TYPE, "1"))
        .collect();
    need(patients.len() <= 1, "at most one patient entity");
    for p in patients {
        need(
            p["what"]["reference"]
                .as_str()
                .is_some_and(|r| r.starts_with("Patient/")),
            "patient entity what is a Patient",
        );
        need(
            has_coding(&p["role"], ENTITY_ROLE, "1"),
            "patient entity role 1",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn row(
        entity_type: &str,
        action: &str,
        old: Option<Value>,
        new: Option<Value>,
    ) -> audit_log::Model {
        audit_log::Model {
            id: Uuid::new_v4(),
            entity_type: entity_type.into(),
            entity_id: Uuid::new_v4(),
            action: action.into(),
            old_value: old,
            new_value: new,
            user_id: Some("dr-evans".into()),
            user_ip: Some("10.0.0.7".into()),
            user_agent: Some("pas-ui/1".into()),
            reason_code: None,
            context: None,
            at: Utc::now().fixed_offset(),
        }
    }

    fn profile(e: &Value) -> Option<&str> {
        e["meta"]["profile"][0].as_str()
    }

    #[test]
    fn a_read_of_a_patient_is_a_patient_read_event() {
        let r = row("patient", "read", None, None);
        let e = to_audit_event(&r, Some("pas-1"));
        assert_eq!(e["action"], "R");
        assert_eq!(e["subtype"][0]["code"], "read");
        assert_eq!(
            profile(&e),
            Some(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead"
            )
        );
        assert!(violations(&e).is_empty(), "{:?}", violations(&e));
        assert_eq!(
            e["entity"][1]["what"]["reference"],
            format!("Patient/{}", r.entity_id)
        );
    }

    #[test]
    fn create_update_and_delete_follow_the_old_and_new_values() {
        let v = Some(json!({"a": 1}));
        for (old, new, action, sub, profile_name) in [
            (None, v.clone(), "C", "create", "IHE.BasicAudit.Create"),
            (v.clone(), v.clone(), "U", "update", "IHE.BasicAudit.Update"),
            (v.clone(), None, "D", "delete", "IHE.BasicAudit.Delete"),
        ] {
            let e = to_audit_event(&row("appointment", "x", old, new), Some("pas-1"));
            assert_eq!(e["action"], action);
            assert_eq!(e["subtype"][0]["code"], sub);
            assert!(profile(&e).unwrap().ends_with(profile_name));
            assert!(violations(&e).is_empty(), "{:?}", violations(&e));
        }
    }

    #[test]
    fn a_row_that_cannot_meet_the_profile_is_exported_without_the_claim() {
        // No client address, no server address, and no user.
        let mut r = row("appointment", "x", None, Some(json!({})));
        r.user_ip = None;
        r.user_id = None;
        let e = to_audit_event(&r, None);
        assert_eq!(profile(&e), None);
        let v = violations(&e);
        assert!(
            v.iter().any(|s| s.contains("client agent network")),
            "{v:?}"
        );
        assert!(
            v.iter().any(|s| s.contains("server agent network")),
            "{v:?}"
        );
        // A domain verb with no values is an Execute event: no subtype, no claim.
        let e = to_audit_event(&row("support_flag", "raise", None, None), Some("pas-1"));
        assert_eq!(e["action"], "E");
        assert_eq!(profile(&e), None);
        assert!(e["subtype"].is_null());
        assert!(!violations(&e).is_empty());
        // An entity with no FHIR type is referenced by urn:uuid.
        assert!(
            e["entity"][0]["what"]["reference"]
                .as_str()
                .unwrap()
                .starts_with("urn:uuid:")
        );
    }

    #[test]
    fn the_checker_catches_a_broken_event() {
        let mut e = to_audit_event(&row("patient", "read", None, None), Some("pas-1"));
        assert!(violations(&e).is_empty());
        e["outcome"] = json!("4");
        e["agent"][2]["requestor"] = json!(false);
        e["entity"][1]["role"] = json!({"system": ENTITY_ROLE, "code": "4"});
        let v = violations(&e);
        assert!(v.contains(&"outcome 0".to_string()), "{v:?}");
        assert!(
            v.contains(&"user agent requestor true".to_string()),
            "{v:?}"
        );
        assert!(v.contains(&"patient entity role 1".to_string()), "{v:?}");
    }
}

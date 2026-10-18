//! Outbound HL7 v2 segment + frame encoders for the v0.44 publisher.
//!
//! These helpers build the message types that PAS emits to a downstream
//! EMR peer over MLLP after an outbox event commits. The data flow is:
//!
//! 1. `Hl7v2MllpPublisher` reads a `DomainEvent` off the outbox.
//! 2. It hydrates the underlying domain rows (Encounter / Appointment /
//!    Bed) from the local DB plus an upstream `Person` (and optionally
//!    `Worker`) snapshot from `person-service` / `worker-service`.
//! 3. It calls one of the `encode_*` helpers below to produce the wire
//!    payload.
//! 4. It MLLP-frames the payload and writes it to the peer.
//!
//! Each builder is pure and synchronous — no I/O, no clock reads except
//! the explicit `now: DateTime<Utc>` parameter — so they're trivial to
//! unit-test. Hydration + transport are the publisher's job.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{Message, Segment, encode_message};
use crate::models::appointment::AppointmentStatus;
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::Bed;

const ENCODING_CHARS: &str = "^~\\&";
const PROCESSING_ID: &str = "P";
const VERSION_ID: &str = "2.5";
const DEFAULT_FAC: &str = "FAC";

/// Format a timestamp as HL7 `YYYYMMDDHHMMSS` in UTC.
pub fn fmt_ts(t: DateTime<Utc>) -> String {
    t.format("%Y%m%d%H%M%S").to_string()
}

/// Build MSH for an outbound message. `msg_type` is the family
/// (e.g. `"ADT"`, `"SIU"`); `trigger` is the event code
/// (e.g. `"A01"`, `"S12"`). `control_id` should be unique per frame —
/// the publisher passes the outbox row id stringified.
pub fn build_msh(
    sender: &str,
    receiver: &str,
    msg_type: &str,
    trigger: &str,
    control_id: &str,
    now: DateTime<Utc>,
) -> Segment {
    Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            ENCODING_CHARS.into(),
            sender.into(),
            DEFAULT_FAC.into(),
            receiver.into(),
            DEFAULT_FAC.into(),
            fmt_ts(now),
            "".into(),
            format!("{msg_type}^{trigger}"),
            control_id.into(),
            PROCESSING_ID.into(),
            VERSION_ID.into(),
        ],
    }
}

/// EVN segment. `recorded` is the event-recording timestamp.
pub fn build_evn(trigger: &str, recorded: DateTime<Utc>) -> Segment {
    Segment {
        name: "EVN".into(),
        fields: vec![trigger.into(), fmt_ts(recorded)],
    }
}

/// Build a minimal PID for a hydrated upstream Person. Reads
/// `identifiers[0]` (CX), `name[0]` (XPN), `birth_date`, `gender`,
/// `address[0]` (XAD), `telecom[0]` (XTN). Missing fields are left
/// empty — the EMR side is expected to tolerate empty optional fields.
pub fn build_pid(person: &crate::upstream::Person) -> Segment {
    let data = &person.data;
    let mrn = data
        .get("identifiers")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (family, given, middle) = name_components(data.get("name").or_else(|| data.get("names")));
    let dob = data
        .get("birth_date")
        .and_then(|v| v.as_str())
        .map(|s| s.replace('-', ""))
        .unwrap_or_default();
    let gender = data
        .get("gender")
        .and_then(|v| v.as_str())
        .map(gender_to_pid8)
        .unwrap_or_default();
    let address = address_xad(data.get("addresses").or_else(|| data.get("address")));
    let phone = telecom_xtn(data.get("telecom").or_else(|| data.get("phone")));
    let pid3 = if mrn.is_empty() {
        String::new()
    } else {
        format!("{mrn}^^^FAC^MR")
    };
    let pid5 = match (family.as_deref(), given.as_deref(), middle.as_deref()) {
        (Some(f), Some(g), Some(m)) => format!("{f}^{g}^{m}"),
        (Some(f), Some(g), None) => format!("{f}^{g}"),
        (Some(f), None, _) => f.to_string(),
        _ => String::new(),
    };
    Segment {
        name: "PID".into(),
        fields: vec![
            "1".into(),
            "".into(),
            pid3,
            "".into(),
            pid5,
            "".into(),
            dob,
            gender,
            "".into(),
            "".into(),
            address,
            "".into(),
            phone,
        ],
    }
}

/// Build PV1 for an encounter + optional bed assignment.
/// PV1-2 is the patient class (I/O/E/P). PV1-3 is the assigned
/// location coded as `WARD^ROOM^BED` (only `BED` is reliable here
/// because the PAS schema only knows the bed code; ward + room
/// are blank and EMRs are expected to resolve them via bed lookup).
/// PV1-19 is the visit id, set to the encounter id stringified.
pub fn build_pv1(enc: &Encounter, bed: Option<&Bed>) -> Segment {
    let class = class_to_pv1_2(enc.class);
    let location = bed.map(|b| format!("^^{}", b.code)).unwrap_or_default();
    let mut fields = vec![
        "1".into(),
        class.into(),
        location,
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        "".into(),
        enc.id.to_string(),
    ];
    if matches!(enc.status, EncounterStatus::Finished)
        && let Some(end) = enc.period_end
    {
        while fields.len() < 45 {
            fields.push(String::new());
        }
        fields.push(fmt_ts(end));
    }
    if matches!(
        enc.status,
        EncounterStatus::InProgress | EncounterStatus::Planned | EncounterStatus::OnLeave
    ) {
        while fields.len() < 44 {
            fields.push(String::new());
        }
        fields.push(fmt_ts(enc.period_start));
    }
    Segment {
        name: "PV1".into(),
        fields,
    }
}

/// SCH segment for an appointment. Populates SCH-1 (placer appt id),
/// SCH-2 (filler appt id, also the appointment uuid), SCH-7 (reason),
/// SCH-11 (appointment timing range), SCH-25 (status).
pub fn build_sch(appt: &crate::models::appointment::Appointment) -> Segment {
    let appt_id = appt.id.to_string();
    let reason = appt.reason.clone().unwrap_or_default();
    let timing = format!(
        "{}^{}^^{}",
        fmt_ts(appt.start_datetime),
        fmt_ts(appt.end_datetime),
        appt_status_to_str(appt.status),
    );
    let status = appt_status_to_str(appt.status).to_string();
    Segment {
        name: "SCH".into(),
        fields: vec![
            appt_id.clone(),
            appt_id,
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            reason,
            "".into(),
            "".into(),
            "".into(),
            timing,
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            status,
        ],
    }
}

/// AIS segment — referenced service. Carries the appointment uuid in
/// AIS-3.
pub fn build_ais(appt: &crate::models::appointment::Appointment) -> Segment {
    Segment {
        name: "AIS".into(),
        fields: vec![
            "1".into(),
            "A".into(), // action = Add
            appt.id.to_string(),
            fmt_ts(appt.start_datetime),
        ],
    }
}

/// AIL segment — appointment location. Populates AIL-3 with the bed
/// code if a bed is supplied (rare for appointments) else leaves blank.
pub fn build_ail(bed: Option<&Bed>) -> Segment {
    let loc = bed.map(|b| b.code.clone()).unwrap_or_default();
    Segment {
        name: "AIL".into(),
        fields: vec!["1".into(), "A".into(), loc],
    }
}

/// AIP segment — appointment personnel. Carries the worker uuid in
/// AIP-3 when known.
pub fn build_aip(worker_id: Option<Uuid>) -> Segment {
    let id = worker_id.map(|i| i.to_string()).unwrap_or_default();
    Segment {
        name: "AIP".into(),
        fields: vec!["1".into(), "A".into(), id],
    }
}

/// Frame-level metadata shared by every `encode_*` entry point.
/// Bundles the MSH-3/-5/-9/-10 inputs + the wall-clock so the
/// `encode_*` signatures stay under clippy's argument-count budget.
#[derive(Debug, Clone, Copy)]
pub struct EncodeContext<'a> {
    pub sender: &'a str,
    pub receiver: &'a str,
    pub trigger: &'a str,
    pub control_id: &'a str,
    pub now: DateTime<Utc>,
}

/// Encode an ADT message: MSH + EVN + PID + PV1.
pub fn encode_adt(
    ctx: EncodeContext<'_>,
    person: &crate::upstream::Person,
    enc: &Encounter,
    bed: Option<&Bed>,
) -> String {
    let msh = build_msh(
        ctx.sender,
        ctx.receiver,
        "ADT",
        ctx.trigger,
        ctx.control_id,
        ctx.now,
    );
    let evn = build_evn(ctx.trigger, ctx.now);
    let pid = build_pid(person);
    let pv1 = build_pv1(enc, bed);
    encode_message(&Message {
        segments: vec![msh, evn, pid, pv1],
    })
}

/// Encode a SIU message: MSH + SCH + PID + AIS + AIL + AIP.
pub fn encode_siu(
    ctx: EncodeContext<'_>,
    person: &crate::upstream::Person,
    appt: &crate::models::appointment::Appointment,
    bed: Option<&Bed>,
) -> String {
    let msh = build_msh(
        ctx.sender,
        ctx.receiver,
        "SIU",
        ctx.trigger,
        ctx.control_id,
        ctx.now,
    );
    let sch = build_sch(appt);
    let pid = build_pid(person);
    let ais = build_ais(appt);
    let ail = build_ail(bed);
    let aip = build_aip(appt.worker_id);
    encode_message(&Message {
        segments: vec![msh, sch, pid, ais, ail, aip],
    })
}

fn name_components(
    v: Option<&serde_json::Value>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(v) = v else {
        return (None, None, None);
    };
    let obj = match v {
        serde_json::Value::Array(a) => a.first(),
        serde_json::Value::Object(_) => Some(v),
        _ => None,
    };
    let Some(obj) = obj else {
        return (None, None, None);
    };
    let family = obj.get("family").and_then(|v| v.as_str()).map(String::from);
    let given = obj.get("given").and_then(|v| match v {
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(String::from),
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    });
    let middle = obj
        .get("given")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(1))
        .and_then(|x| x.as_str())
        .map(String::from);
    (family, given, middle)
}

fn address_xad(v: Option<&serde_json::Value>) -> String {
    let Some(v) = v else {
        return String::new();
    };
    let obj = match v {
        serde_json::Value::Array(a) => a.first(),
        serde_json::Value::Object(_) => Some(v),
        _ => None,
    };
    let Some(obj) = obj else {
        return String::new();
    };
    let line = obj
        .get("line")
        .and_then(|v| match v {
            serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(String::from),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let city = obj.get("city").and_then(|v| v.as_str()).unwrap_or("");
    let state = obj.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let postal = obj
        .get("postal_code")
        .or_else(|| obj.get("postalCode"))
        .or_else(|| obj.get("zip"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let country = obj.get("country").and_then(|v| v.as_str()).unwrap_or("");
    format!("{line}^^{city}^{state}^{postal}^{country}")
}

fn telecom_xtn(v: Option<&serde_json::Value>) -> String {
    let Some(v) = v else {
        return String::new();
    };
    match v {
        serde_json::Value::Array(a) => a
            .iter()
            .find_map(|t| t.get("value").and_then(|x| x.as_str()).map(String::from))
            .unwrap_or_default(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn gender_to_pid8(g: &str) -> String {
    match g.to_ascii_lowercase().as_str() {
        "male" | "m" => "M".to_string(),
        "female" | "f" => "F".to_string(),
        "other" | "o" => "O".to_string(),
        "unknown" | "u" => "U".to_string(),
        _ => "".to_string(),
    }
}

fn class_to_pv1_2(c: EncounterClass) -> &'static str {
    match c {
        EncounterClass::Inpatient => "I",
        EncounterClass::Outpatient => "O",
        EncounterClass::Emergency => "E",
        EncounterClass::DayCase => "C",
        EncounterClass::HomeCare => "H",
        EncounterClass::Virtual => "V",
    }
}

fn appt_status_to_str(s: AppointmentStatus) -> &'static str {
    match s {
        AppointmentStatus::Proposed => "Proposed",
        AppointmentStatus::Booked => "Booked",
        AppointmentStatus::Arrived => "Arrived",
        AppointmentStatus::Fulfilled => "Fulfilled",
        AppointmentStatus::Cancelled => "Cancelled",
        AppointmentStatus::NoShow => "NoShow",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    fn sample_person() -> crate::upstream::Person {
        let mut map = serde_json::Map::new();
        map.insert(
            "identifiers".into(),
            json!([{ "system": "MR", "value": "MRN-99" }]),
        );
        map.insert(
            "name".into(),
            json!({ "family": "Doe", "given": ["Jane", "M"] }),
        );
        map.insert("birth_date".into(), json!("1990-01-15"));
        map.insert("gender".into(), json!("female"));
        map.insert(
            "addresses".into(),
            json!([{ "line": ["123 Elm"], "city": "Springfield", "state": "IL", "postal_code": "62701", "country": "US" }]),
        );
        map.insert(
            "telecom".into(),
            json!([{ "system": "phone", "value": "(555)555-0100" }]),
        );
        crate::upstream::Person {
            id: Uuid::nil(),
            data: map,
        }
    }

    #[test]
    fn test_fmt_ts_pads_to_14_chars() {
        let t = at(2026, 1, 2, 3, 4, 5);
        assert_eq!(fmt_ts(t), "20260102030405");
    }

    #[test]
    fn test_build_msh_carries_type_and_control_id() {
        let s = build_msh(
            "PAS",
            "EMR",
            "ADT",
            "A01",
            "OUT-1",
            at(2026, 5, 31, 12, 0, 0),
        );
        let wire = encode_message(&Message { segments: vec![s] });
        assert!(
            wire.starts_with("MSH|^~\\&|PAS|FAC|EMR|FAC|20260531120000||ADT^A01|OUT-1|P|2.5\r")
        );
    }

    #[test]
    fn test_build_evn_shape() {
        let s = build_evn("A01", at(2026, 5, 31, 12, 0, 0));
        let wire = encode_message(&Message { segments: vec![s] });
        assert!(wire.starts_with("EVN|A01|20260531120000"));
    }

    #[test]
    fn test_build_pid_renders_known_fields() {
        let p = sample_person();
        let s = build_pid(&p);
        let wire = encode_message(&Message { segments: vec![s] });
        assert!(wire.contains("|MRN-99^^^FAC^MR|"));
        assert!(wire.contains("|Doe^Jane^M|"));
        assert!(wire.contains("|19900115|F|"));
        assert!(wire.contains("123 Elm^^Springfield^IL^62701^US"));
        assert!(wire.contains("(555)555-0100"));
    }

    #[test]
    fn test_build_pid_tolerates_empty_person() {
        let p = crate::upstream::Person {
            id: Uuid::nil(),
            data: serde_json::Map::new(),
        };
        let s = build_pid(&p);
        let wire = encode_message(&Message { segments: vec![s] });
        // Just confirm encoding doesn't panic and the segment exists.
        assert!(wire.starts_with("PID|1|"));
    }

    #[test]
    fn test_build_pv1_includes_bed_code_and_class() {
        let enc = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        let bed = Bed::new(Uuid::new_v4(), "Bed 1".into(), "B-101".into());
        let s = build_pv1(&enc, Some(&bed));
        let wire = encode_message(&Message { segments: vec![s] });
        assert!(wire.starts_with("PV1|1|I|^^B-101|"));
    }

    fn ctx<'a>(trigger: &'a str, control_id: &'a str, now: DateTime<Utc>) -> EncodeContext<'a> {
        EncodeContext {
            sender: "PAS",
            receiver: "EMR",
            trigger,
            control_id,
            now,
        }
    }

    #[test]
    fn test_encode_adt_a01_round_trip() {
        let person = sample_person();
        let enc = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        let bed = Bed::new(Uuid::new_v4(), "Bed".into(), "B-1".into());
        let wire = encode_adt(
            ctx("A01", "OUT-42", at(2026, 5, 31, 12, 0, 0)),
            &person,
            &enc,
            Some(&bed),
        );
        assert!(wire.contains("|ADT^A01|OUT-42|P|2.5\r"));
        assert!(wire.contains("EVN|A01|20260531120000"));
        assert!(wire.contains("PID|1|"));
        assert!(wire.contains("PV1|1|I|^^B-1|"));
    }

    #[test]
    fn test_encode_adt_a03_no_bed_still_emits_pv1() {
        let person = sample_person();
        let mut enc = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        enc.status = EncounterStatus::Finished;
        enc.period_end = Some(at(2026, 5, 31, 18, 0, 0));
        let wire = encode_adt(
            ctx("A03", "OUT-7", at(2026, 5, 31, 18, 0, 5)),
            &person,
            &enc,
            None,
        );
        assert!(wire.contains("|ADT^A03|OUT-7|P|2.5\r"));
        assert!(wire.contains("\rPV1|1|I||"));
    }

    #[test]
    fn test_encode_siu_s12_includes_sch_pid_ais() {
        let person = sample_person();
        let appt = crate::models::appointment::Appointment::new(
            Uuid::new_v4(),
            at(2026, 6, 1, 9, 0, 0),
            at(2026, 6, 1, 10, 0, 0),
        );
        let wire = encode_siu(
            ctx("S12", "OUT-3", at(2026, 5, 31, 12, 0, 0)),
            &person,
            &appt,
            None,
        );
        assert!(wire.contains("|SIU^S12|OUT-3|P|2.5\r"));
        assert!(wire.contains("\rSCH|"));
        assert!(wire.contains("\rPID|1|"));
        assert!(wire.contains("\rAIS|1|A|"));
        assert!(wire.contains("\rAIL|1|A|"));
        assert!(wire.contains("\rAIP|1|A|"));
    }

    #[test]
    fn test_gender_to_pid8_handles_variants() {
        assert_eq!(gender_to_pid8("Male"), "M");
        assert_eq!(gender_to_pid8("female"), "F");
        assert_eq!(gender_to_pid8("OTHER"), "O");
        assert_eq!(gender_to_pid8("u"), "U");
        assert_eq!(gender_to_pid8(""), "");
        assert_eq!(gender_to_pid8("garbage"), "");
    }

    #[test]
    fn test_class_to_pv1_2_maps_all_classes() {
        assert_eq!(class_to_pv1_2(EncounterClass::Inpatient), "I");
        assert_eq!(class_to_pv1_2(EncounterClass::Outpatient), "O");
        assert_eq!(class_to_pv1_2(EncounterClass::Emergency), "E");
        assert_eq!(class_to_pv1_2(EncounterClass::DayCase), "C");
        assert_eq!(class_to_pv1_2(EncounterClass::HomeCare), "H");
        assert_eq!(class_to_pv1_2(EncounterClass::Virtual), "V");
    }
}

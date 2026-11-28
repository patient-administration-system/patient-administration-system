//! HUB-7: golden tests for the HL7 v2 mappings. No database.
//!
//! Each example message in `examples/` is read three ways: as sent, with its PID removed, and with an extra
//! Z-segment at the end. What the PAS takes from it (the person it would create, the MRN, the bed, a merge's old MRN, a schedule, charges) and the ACK it answers with are compared with `tests/golden/hl7/<example>.json`. The Z-segment must
//! change nothing.
//!
//! **A golden file is the record of what the PAS does.** If a mapping change makes this fail, the change is
//! either a defect or a decision: re-capture only with a spec change (an FR, an NFR, or a row in
//! `spec/deviations.md`) named in the commit message, with `PAS_GOLDEN_WRITE=1 cargo test --test
//! hub7_hl7_golden_test`. Never re-capture to make the test pass.
use patient_administration_system::hl7v2::ack::{AckCode, Inbound, ack_to};
use patient_administration_system::hl7v2::pid_translation::{
    extract_bed_code_from_pv1, extract_mrn_from_mrg, extract_mrn_from_pid, parse_ft1, parse_sch,
    pid_to_person_create,
};
use patient_administration_system::hl7v2::routing::{ROUTES, refusal};
use patient_administration_system::hl7v2::{Message, SegmentRead, parse_batch, parse_message};
use serde_json::{Value, json};
use std::path::PathBuf;

const EXAMPLES: [&str; 9] = [
    "hl7-adt-a01",
    "hl7-adt-a02",
    "hl7-adt-a03",
    "hl7-adt-a04",
    "hl7-adt-a08",
    "hl7-adt-a28",
    "hl7-adt-a40",
    "hl7-siu-s12",
    "hl7-dft-p03",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn example(name: &str) -> String {
    std::fs::read_to_string(root().join(format!("examples/{name}.txt"))).unwrap()
}

/// What the PAS reads from one message.
fn reading(msg: &Message) -> Value {
    let pid = msg.segment("PID");
    let pv1 = msg.segment("PV1");
    json!({
        "type": msg.segment("MSH").map(|m| m.raw(9)),
        "control_id": msg.segment("MSH").map(|m| m.raw(10)),
        "mrn": pid.and_then(extract_mrn_from_pid),
        "person": pid.map(|p| serde_json::to_value(pid_to_person_create(p)).unwrap()),
        "bed_code": pv1.and_then(extract_bed_code_from_pv1),
        "merged_from_mrn": msg.segment("MRG").and_then(extract_mrn_from_mrg),
        "schedule": msg.segment("SCH").map(|s| {
            let sch = parse_sch(s);
            json!({
                "placer_id": sch.placer_id,
                "filler_id": sch.filler_id,
                "reason": sch.reason,
                "duration_minutes": sch.duration_minutes,
                "start": sch.start,
            })
        }),
        "charges": msg
            .segments
            .iter()
            .filter(|s| s.name == "FT1")
            .map(|s| match parse_ft1(s) {
                Ok(f) => json!({
                    "set_id": f.set_id,
                    "posted_at": f.posted_at,
                    "type": f.transaction_type,
                    "code": f.code,
                    "description": f.description,
                    "amount": f.amount,
                    "currency": f.currency,
                }),
                Err(e) => json!({ "error": e.to_string() }),
            })
            .collect::<Vec<_>>(),
    })
}

/// The ACK with its two moving parts (the time and our own control id) fixed, so it can be compared.
fn normalised_ack(msg: &Message) -> String {
    let ack = ack_to(&Inbound::of(msg), AckCode::Accept, Some("golden"));
    let mut fields: Vec<String> = ack
        .split('\r')
        .next()
        .unwrap()
        .split('|')
        .map(str::to_string)
        .collect();
    fields[6] = "<time>".into();
    fields[9] = "<ack id>".into();
    let rest: Vec<&str> = ack.split('\r').skip(1).collect();
    format!("{}\r{}", fields.join("|"), rest.join("\r"))
}

/// `text` without its `segment` lines. The examples end lines with `\n`, as a file does; senders use `\r`.
fn without(text: &str, segment: &str) -> String {
    text.split(['\r', '\n'])
        .filter(|s| !s.is_empty() && !s.starts_with(&format!("{segment}|")))
        .collect::<Vec<_>>()
        .join("\r")
}

fn with_z_segment(text: &str) -> String {
    format!(
        "{}\rZPV|1|site extension^value\r",
        text.trim_end_matches(['\r', '\n'])
    )
}

fn check_or_write(name: &str, actual: &Value) {
    let path = root().join(format!("tests/golden/hl7/{name}.json"));
    let pretty = serde_json::to_string_pretty(actual).unwrap() + "\n";
    if std::env::var("PAS_GOLDEN_WRITE").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, pretty).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("no golden file {}; see this file's header", path.display()));
    assert_eq!(
        pretty, expected,
        "{name}: the HL7 v2 mapping changed; a defect, or a decision that needs a spec change (see the header)"
    );
}

#[test]
fn each_example_maps_as_its_golden_file_says() {
    for name in EXAMPLES {
        let text = example(name);
        let sent = parse_message(&text).unwrap();
        let no_pid = parse_message(&without(&text, "PID")).unwrap();
        let z = parse_message(&with_z_segment(&text)).unwrap();
        assert_eq!(
            reading(&z),
            reading(&sent),
            "{name}: a Z-segment changed what was read"
        );
        assert_eq!(
            normalised_ack(&z),
            normalised_ack(&sent),
            "{name}: a Z-segment changed the ACK"
        );
        check_or_write(
            name,
            &json!({
                "as_sent": reading(&sent),
                "without_pid": reading(&no_pid),
                "ack": normalised_ack(&sent),
            }),
        );
    }
}

#[test]
fn the_batch_example_reads_as_its_golden_file_says() {
    let batch = parse_batch(&example("hl7-batch-adt")).unwrap();
    let messages: Vec<Value> = batch.messages.iter().map(reading).collect();
    check_or_write(
        "hl7-batch-adt",
        &json!({
            "bhs_control_id": batch.bhs.as_ref().map(|b| b.raw(11)),
            "messages": messages,
        }),
    );
}

/// DEV-4: which HTTP paths each example is accepted at (exactly one), and the refusal everywhere else, with
/// the refusal ACK for a lab result posted as an admission.
#[test]
fn each_example_is_accepted_only_at_its_own_path() {
    let mut accepted = serde_json::Map::new();
    for name in EXAMPLES {
        let msg = parse_message(&example(name)).unwrap();
        let at: Vec<&str> = ROUTES
            .iter()
            .map(|(_, _, p)| *p)
            .filter(|p| refusal(p, &msg).is_none())
            .collect();
        assert_eq!(at.len(), 1, "{name} is accepted at {at:?}");
        accepted.insert(name.to_string(), json!(at[0]));
    }
    let lab = parse_message(
        "MSH|^~\\&|LAB|HOSP|PAS|HOSP|20261128120000||ORU^R01|LAB-1|P|2.5\rPID|1||X^^^HOSP^MR\r",
    )
    .unwrap();
    let why = refusal("/api/hl7/v2/admit", &lab).unwrap();
    let mut ack: Vec<String> = ack_to(&Inbound::of(&lab), AckCode::Reject, Some(&why))
        .split('\r')
        .map(str::to_string)
        .collect();
    let mut msh: Vec<String> = ack[0].split('|').map(str::to_string).collect();
    msh[6] = "<time>".into();
    msh[9] = "<ack id>".into();
    ack[0] = msh.join("|");
    check_or_write(
        "hl7-type-check",
        &json!({ "accepted_at": accepted, "lab_result_posted_as_admission": ack.join("\r") }),
    );
}

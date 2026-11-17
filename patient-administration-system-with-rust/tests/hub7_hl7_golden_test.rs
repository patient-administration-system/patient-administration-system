//! HUB-7: golden tests for the HL7 v2 mappings. No database.
//!
//! Each example message in `examples/` is read three ways: as sent, with its PID removed, and with an extra
//! Z-segment at the end. What the PAS takes from it (the person it would create, the MRN, the bed) and the ACK it answers with are compared with `tests/golden/hl7/<example>.json`. The Z-segment must
//! change nothing.
//!
//! **A golden file is the record of what the PAS does.** If a mapping change makes this fail, the change is
//! either a defect or a decision: re-capture only with a spec change (an FR, an NFR, or a row in
//! `spec/deviations.md`) named in the commit message, with `PAS_GOLDEN_WRITE=1 cargo test --test
//! hub7_hl7_golden_test`. Never re-capture to make the test pass.
use patient_administration_system::hl7v2::ack::{AckCode, Inbound, ack_to};
use patient_administration_system::hl7v2::pid_translation::{
    extract_bed_code_from_pv1, extract_mrn_from_pid, pid_to_person_create,
};
use patient_administration_system::hl7v2::{Message, SegmentRead, parse_batch, parse_message};
use serde_json::{Value, json};
use std::path::PathBuf;

const EXAMPLES: [&str; 4] = ["hl7-adt-a01", "hl7-adt-a02", "hl7-adt-a03", "hl7-adt-a28"];

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

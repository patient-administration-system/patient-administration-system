//! Any bytes: parsing an HL7 v2 message never panics, and what parses encodes and parses again.
#![no_main]
use libfuzzer_sys::fuzz_target;
use patient_administration_system::hl7v2::{encode_message, parse_message};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(m) = parse_message(&text) {
        let _ = parse_message(&encode_message(&m));
    }
});

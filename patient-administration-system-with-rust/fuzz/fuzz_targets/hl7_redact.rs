//! Any text the HL7 v2 log redactor could be given (HUB-5, NFR-33): it never panics, and a marker in a segment
//! after the header never comes out, unless the input already held it (it may sit in a kept header field).
#![no_main]
use libfuzzer_sys::fuzz_target;
use patient_administration_system::hl7v2::redact::redact;

const NEEDLE: &str = "NEEDLE7391";

fuzz_target!(|text: &str| {
    let out = redact(&format!("{text}\rZZZ|{NEEDLE}|x^{NEEDLE}"));
    assert!(text.contains(NEEDLE) || !out.contains(NEEDLE), "{out:?}");
    let _ = redact(text);
});

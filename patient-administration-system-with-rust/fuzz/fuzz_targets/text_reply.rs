//! Any text a patient could send: reading it as a reply never panics.
#![no_main]
use libfuzzer_sys::fuzz_target;
use patient_administration_system::inbound::parser::parse;

fuzz_target!(|text: &str| {
    let _ = parse(text);
});

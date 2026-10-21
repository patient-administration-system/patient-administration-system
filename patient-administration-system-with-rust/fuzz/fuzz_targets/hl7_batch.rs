//! Any bytes: splitting an HL7 v2 batch never panics.
#![no_main]
use libfuzzer_sys::fuzz_target;
use patient_administration_system::hl7v2::batch::parse_batch;

fuzz_target!(|data: &[u8]| {
    let _ = parse_batch(&String::from_utf8_lossy(data));
});

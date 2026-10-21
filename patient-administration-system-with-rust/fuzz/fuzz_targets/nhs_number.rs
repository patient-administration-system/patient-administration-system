//! Any text: checking it as an NHS number never panics, and a number that passes still passes when
//! written the usual way.
#![no_main]
use libfuzzer_sys::fuzz_target;
use patient_administration_system::validation::{format_nhs_number, validate_nhs_number};

fuzz_target!(|text: &str| {
    if let Ok(n) = validate_nhs_number(text) {
        assert_eq!(validate_nhs_number(&format_nhs_number(&n)).ok(), Some(n));
    }
});

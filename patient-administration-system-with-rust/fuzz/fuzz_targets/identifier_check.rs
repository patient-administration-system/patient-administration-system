#![no_main]
//! Every identifier scheme's check: no input panics it, and a value it accepts, once formatted, is accepted
//! again as the same value.

use libfuzzer_sys::fuzz_target;
use patient_administration_system::identifiers::SCHEMES;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        for scheme in SCHEMES {
            if let Ok(v) = (scheme.check)(text) {
                assert_eq!((scheme.check)(&(scheme.format)(&v)).ok(), Some(v));
            }
        }
    }
});

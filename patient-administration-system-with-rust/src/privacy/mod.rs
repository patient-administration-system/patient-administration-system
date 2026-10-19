//! privacy
//!
//! Data masking helpers. The patient-specific `mask_patient` +
//! `export_patient` surface that lived here through v0.43.0 is removed
//! in v0.44.0 Phase 2 because identity moved to `person-service`
//! (which owns masking + export via its own `/api/persons/{id}/masked`
//! and `/api/persons/{id}/export` endpoints). Phase 3 reintroduces a
//! thin `export_patient_combined` helper that joins the upstream's
//! GDPR export with PAS-owned related rows (encounters, appointments,
//! billing, consents).
//!
//! [`mask_value`] and [`mask_contact_point`] survive — they're generic
//! string helpers used by code that still touches free-form telecom
//! values inside PAS-owned tables.

pub mod consent;

use crate::models::{ContactPoint, ContactPointSystem};

/// Mask all but the last `keep_tail` characters of `value` with `*`.
///
/// If `keep_tail >= value.chars().count()`, the entire string is masked.
/// Operates on Unicode scalar values, not bytes, so multi-byte characters
/// count as one.
pub fn mask_value(value: &str, keep_tail: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= keep_tail {
        return "*".repeat(chars.len());
    }
    let masked_len = chars.len() - keep_tail;
    let mut out: String = std::iter::repeat_n('*', masked_len).collect();
    out.extend(chars.iter().skip(masked_len));
    out
}

/// Mask one [`ContactPoint`] according to its system.
pub fn mask_contact_point(c: &ContactPoint) -> ContactPoint {
    let masked = match c.system {
        ContactPointSystem::Email => {
            if let Some(at) = c.value.find('@') {
                let local = &c.value[..at];
                let rest = &c.value[at..];
                format!("{}{rest}", mask_value(local, 1))
            } else {
                mask_value(&c.value, 0)
            }
        }
        ContactPointSystem::Phone | ContactPointSystem::Sms | ContactPointSystem::Fax => {
            mask_value(&c.value, 4)
        }
        _ => mask_value(&c.value, 0),
    };
    ContactPoint {
        value: masked,
        ..c.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_value_keeps_tail() {
        assert_eq!(mask_value("abcdef", 2), "****ef");
    }

    #[test]
    fn test_mask_value_short_string_fully_masked() {
        assert_eq!(mask_value("ab", 4), "**");
    }

    #[test]
    fn test_mask_value_unicode() {
        assert_eq!(mask_value("café", 2), "**fé");
    }

    #[test]
    fn test_mask_contact_point_email() {
        let cp = ContactPoint {
            system: ContactPointSystem::Email,
            value: "alice@example.com".into(),
            use_type: None,
        };
        let m = mask_contact_point(&cp);
        assert_eq!(m.value, "****e@example.com");
    }

    #[test]
    fn test_mask_contact_point_phone_keeps_last_4() {
        let cp = ContactPoint {
            system: ContactPointSystem::Phone,
            value: "5551234567".into(),
            use_type: None,
        };
        let m = mask_contact_point(&cp);
        assert_eq!(m.value, "******4567");
    }
}

//! Which HL7 v2 message type goes to which handler (DEV-4, audit PA-16).
//!
//! One table, read three ways: the MLLP listener picks the HTTP path for a message from it
//! ([`path_for`]), and each mutating HTTP path accepts only the type the table gives it ([`refusal`]), so a
//! message an interface engine posts to the wrong address is refused before anything changes. A batch picks
//! each message's handler from the same pairs.

use crate::hl7v2::{Message, SegmentRead};

/// (MSH-9.1 message code, MSH-9.2 trigger event, the HTTP path that handles it). The only accepted pairs.
pub const ROUTES: [(&str, &str, &str); 24] = [
    ("ADT", "A28", "/api/hl7/v2/patient"),
    ("ADT", "A01", "/api/hl7/v2/admit"),
    ("ADT", "A02", "/api/hl7/v2/transfer"),
    ("ADT", "A03", "/api/hl7/v2/discharge"),
    ("ADT", "A04", "/api/hl7/v2/register"),
    ("ADT", "A05", "/api/hl7/v2/pre-admit"),
    ("ADT", "A06", "/api/hl7/v2/change-to-inpatient"),
    ("ADT", "A07", "/api/hl7/v2/change-to-outpatient"),
    ("ADT", "A08", "/api/hl7/v2/update"),
    ("ADT", "A11", "/api/hl7/v2/cancel-admit"),
    ("ADT", "A12", "/api/hl7/v2/cancel-transfer"),
    ("ADT", "A13", "/api/hl7/v2/cancel-discharge"),
    ("ADT", "A21", "/api/hl7/v2/leave-start"),
    ("ADT", "A22", "/api/hl7/v2/leave-end"),
    ("ADT", "A23", "/api/hl7/v2/delete-patient"),
    ("ADT", "A38", "/api/hl7/v2/cancel-pre-admit"),
    ("ADT", "A40", "/api/hl7/v2/merge"),
    ("DFT", "P03", "/api/hl7/v2/dft"),
    ("MFN", "M02", "/api/hl7/v2/mfn-staff"),
    ("MFN", "M05", "/api/hl7/v2/mfn-location"),
    ("SIU", "S12", "/api/hl7/v2/schedule-book"),
    ("SIU", "S13", "/api/hl7/v2/schedule-reschedule"),
    ("SIU", "S14", "/api/hl7/v2/schedule-modify"),
    ("SIU", "S15", "/api/hl7/v2/schedule-cancel"),
];

/// MSH-9.1 and MSH-9.2 (`ADT^A01^ADT_A01` is `("ADT", "A01")`); empty strings when there is no MSH.
#[must_use]
pub fn message_type(msg: &Message) -> (String, String) {
    let Some(msh) = msg.segment("MSH") else {
        return (String::new(), String::new());
    };
    let field = msh.raw(9);
    let mut parts = field.split('^');
    let code = parts.next().unwrap_or("").to_string();
    let event = parts.next().unwrap_or("").to_string();
    (code, event)
}

/// The HTTP path that handles this type, or `None` for a type the PAS does not handle.
#[must_use]
pub fn path_for(code: &str, event: &str) -> Option<&'static str> {
    ROUTES
        .iter()
        .find(|(c, e, _)| *c == code && *e == event)
        .map(|(_, _, p)| *p)
}

/// The type a mutating HTTP path accepts, as `ADT^A01`, or `None` for a path not in the table.
#[must_use]
pub fn accepted_at(path: &str) -> Option<String> {
    ROUTES
        .iter()
        .find(|(_, _, p)| *p == path)
        .map(|(c, e, _)| format!("{c}^{e}"))
}

/// Why a message posted to `path` is refused, or `None` when MSH-9 is the type that path handles. The text
/// is an MSA-3 (DEV-2): it names the field and what the path accepts, never what the sender put in MSH-9.
#[must_use]
pub fn refusal(path: &str, msg: &Message) -> Option<String> {
    let (code, event) = message_type(msg);
    let Some(accepted) = accepted_at(path) else {
        return Some("MSH-9: this path takes no HL7 v2 message".to_string());
    };
    if path_for(&code, &event) == Some(path) {
        return None;
    }
    Some(format!(
        "MSH-9 does not match this path, which takes {accepted}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::parse_message;

    fn msg(kind: &str) -> Message {
        parse_message(&format!("MSH|^~\\&|S|F|PAS|H|20261128||{kind}|1|P|2.5\r")).unwrap()
    }

    #[test]
    fn each_path_takes_its_own_type_and_no_other() {
        for (code, event, path) in ROUTES {
            assert_eq!(
                refusal(path, &msg(&format!("{code}^{event}^{code}_{event}"))),
                None,
                "{path}"
            );
            for (other_code, other_event, other_path) in ROUTES {
                if other_path != path {
                    let text = refusal(path, &msg(&format!("{other_code}^{other_event}"))).unwrap();
                    assert!(text.starts_with("MSH-9 does not match"), "{text}");
                    assert!(text.len() <= 80, "{text}");
                }
            }
        }
    }

    #[test]
    fn an_unhandled_type_a_missing_trigger_and_an_unknown_path_are_refused() {
        assert!(refusal("/api/hl7/v2/admit", &msg("ORU^R01")).is_some());
        assert!(refusal("/api/hl7/v2/update", &msg("ADT^A31")).is_some());
        assert!(refusal("/api/hl7/v2/admit", &msg("ADT")).is_some());
        assert!(refusal("/api/hl7/v2/admit", &msg("")).is_some());
        assert_eq!(
            refusal("/api/hl7/v2/elsewhere", &msg("ADT^A01")).as_deref(),
            Some("MSH-9: this path takes no HL7 v2 message")
        );
    }

    #[test]
    fn the_refusal_names_what_the_path_takes_and_not_what_was_sent() {
        let text = refusal("/api/hl7/v2/admit", &msg("ORU^R01")).unwrap();
        assert_eq!(text, "MSH-9 does not match this path, which takes ADT^A01");
        assert!(!text.contains("ORU"));
    }

    #[test]
    fn every_pair_and_every_path_appears_once() {
        let mut pairs: Vec<_> = ROUTES.iter().map(|(c, e, _)| (c, e)).collect();
        let mut paths: Vec<_> = ROUTES.iter().map(|(_, _, p)| p).collect();
        pairs.sort();
        pairs.dedup();
        paths.sort();
        paths.dedup();
        assert_eq!(pairs.len(), ROUTES.len());
        assert_eq!(paths.len(), ROUTES.len());
    }
}

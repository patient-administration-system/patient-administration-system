//! HL7 v2 ACK builder (NFR-31).
//!
//! An ACK answers one inbound message, so it is built from that message's header ([`Inbound`]):
//!
//! - **Swapped:** MSH-3/4 of the ACK are the inbound MSH-5/6 (we are the receiver it named, or `PAS` when it
//!   named none), and MSH-5/6 are the inbound MSH-3/4.
//! - **Copied:** MSH-11 (processing id), so a test (`T`) or debug (`D`) message is never acknowledged as
//!   production, and MSH-12 (version); `P` and `2.5` only when the inbound left them empty.
//! - **Its own id:** MSH-10 is a new control id; MSA-2 echoes the inbound MSH-10.
//! - **Safe text:** MSA-3 is one line (line breaks become spaces), at most [`MSA3_MAX`] characters, and
//!   escaped, so no `|`, `^`, `~`, `\` or `&` in a reason can change the ACK's structure.
//!
//! Values copied from the inbound header are its wire text (`raw`), so they are already valid in the same
//! position and are not escaped again.

use jiff::Timestamp;

use super::{Message, SegmentRead, encode_message, escape::escape_value, message, segment};

/// The most characters MSA-3 (text message) may hold: its length in HL7 v2.5.
pub const MSA3_MAX: usize = 80;

/// Who we are when the inbound message names no receiving application.
pub const PAS_APP: &str = "PAS";

/// MSA-1 acknowledgement code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckCode {
    /// `AA`: Application Accept.
    Accept,
    /// `AE`: Application Error (the message was well-formed but could not be processed).
    AppError,
    /// `AR`: Application Reject (the message was rejected before processing).
    Reject,
}

impl AckCode {
    pub fn as_str(self) -> &'static str {
        match self {
            AckCode::Accept => "AA",
            AckCode::AppError => "AE",
            AckCode::Reject => "AR",
        }
    }

    /// The code an ACK's MSA-1 names, if it is one of the three.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "AA" => Some(AckCode::Accept),
            "AE" => Some(AckCode::AppError),
            "AR" => Some(AckCode::Reject),
            _ => None,
        }
    }
}

/// The parts of an inbound MSH that its ACK needs, as wire text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbound {
    /// MSH-3.
    pub sending_app: String,
    /// MSH-4.
    pub sending_facility: String,
    /// MSH-5.
    pub receiving_app: String,
    /// MSH-6.
    pub receiving_facility: String,
    /// MSH-9.2, the trigger event (`A01`), for the ACK's own MSH-9.
    pub event: String,
    /// MSH-10.
    pub control_id: String,
    /// MSH-11.
    pub processing_id: String,
    /// MSH-12.
    pub version: String,
}

impl Inbound {
    /// The header of `msg`; every field empty when it has no MSH.
    pub fn of(msg: &Message) -> Self {
        let Some(msh) = msg.segment("MSH") else {
            return Self::default();
        };
        Self {
            sending_app: msh.raw(3),
            sending_facility: msh.raw(4),
            receiving_app: msh.raw(5),
            receiving_facility: msh.raw(6),
            event: msh.component_text(9, 2),
            control_id: msh.raw(10),
            processing_id: msh.raw(11),
            version: msh.raw(12),
        }
    }

    /// For a message that could not be parsed at all: nothing is known about it.
    pub fn unknown() -> Self {
        Self::default()
    }
}

/// A new control id for an ACK's MSH-10: 20 hex digits.
fn new_control_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..20].to_string()
}

/// MSA-3 text: one line, at most [`MSA3_MAX`] characters, escaped.
pub fn msa3_text(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .take(MSA3_MAX)
        .collect();
    escape_value(line.trim_end())
}

fn or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

/// The ACK for `inbound`, with `code` and an optional reason in MSA-3.
pub fn ack_to(inbound: &Inbound, code: AckCode, text: Option<&str>) -> String {
    let now = Timestamp::now().strftime("%Y%m%d%H%M%S").to_string();
    let message_type = if inbound.event.is_empty() {
        "ACK".to_string()
    } else {
        format!("ACK^{}^ACK", escape_value(&inbound.event))
    };
    let msh = segment(
        "MSH",
        vec![
            "|".into(),
            "^~\\&".into(),
            or(&inbound.receiving_app, PAS_APP).into(),
            inbound.receiving_facility.clone(),
            inbound.sending_app.clone(),
            inbound.sending_facility.clone(),
            now,
            String::new(),
            message_type,
            new_control_id(),
            or(&inbound.processing_id, "P").into(),
            or(&inbound.version, "2.5").into(),
        ],
    );
    let mut msa_fields = vec![code.as_str().into(), inbound.control_id.clone()];
    if let Some(t) = text {
        msa_fields.push(msa3_text(t));
    }
    let msa = segment("MSA", msa_fields);
    encode_message(&message(vec![msh, msa]))
}

/// An ACK from the bare names: `receiving_app` sent message `message_control_id` to `sending_app`. Kept for
/// callers that hold no parsed header (a frame that would not parse); prefer [`ack_to`].
pub fn ack(
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
    code: AckCode,
    diagnostics: Option<&str>,
) -> String {
    let inbound = Inbound {
        sending_app: receiving_app.into(),
        receiving_app: sending_app.into(),
        control_id: message_control_id.into(),
        ..Inbound::default()
    };
    ack_to(&inbound, code, diagnostics)
}

/// MSA-1 and MSA-3 (unescaped) of an ACK we built, for replaying it to a resent message (HUB-2).
pub fn ack_code_and_text(ack: &str) -> Option<(AckCode, Option<String>)> {
    let msg = super::parse_message(ack).ok()?;
    let msa = msg.segment("MSA")?;
    let code = AckCode::parse(&msa.raw(1))?;
    let text = msa.raw(3);
    let text = (!text.is_empty()).then(|| super::escape::unescape_value(&text));
    Some((code, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::parse_message;

    fn inbound(msh: &str) -> Inbound {
        Inbound::of(&parse_message(&format!("{msh}\rPID|1||MRN-1\r")).unwrap())
    }

    fn fields(ack: &str, name: &str) -> Vec<String> {
        let msg = parse_message(ack).unwrap();
        let seg = msg.segment(name).unwrap();
        (1..=12).map(|n| seg.raw(n)).collect()
    }

    #[test]
    fn every_code_parses_back_to_itself_and_nothing_else_parses() {
        for code in [AckCode::Accept, AckCode::AppError, AckCode::Reject] {
            assert_eq!(AckCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(AckCode::parse("CA"), None);
        assert_eq!(AckCode::parse(""), None);
    }

    #[test]
    fn an_ack_from_the_bare_names_is_from_the_sender_to_the_receiver() {
        let a = ack("PASAPP", "EMR", "CTL-1", AckCode::Reject, Some("bad"));
        let msh = fields(&a, "MSH");
        assert_eq!((msh[2].as_str(), msh[4].as_str()), ("PASAPP", "EMR"));
        let msa = fields(&a, "MSA");
        assert_eq!(&msa[..3], ["AR", "CTL-1", "bad"]);
    }

    #[test]
    fn swaps_sender_and_receiver_and_copies_processing_id_and_version() {
        let i = inbound("MSH|^~\\&|EMR|WARDS|PASAPP|HOSP|20260523120000||ADT^A01|MSG001|T|2.4");
        let a = ack_to(&i, AckCode::Accept, None);
        let msh = fields(&a, "MSH");
        // raw(n) of MSH: 1 is the field separator, 2 the encoding characters.
        assert_eq!(&msh[2..6], ["PASAPP", "HOSP", "EMR", "WARDS"]);
        assert_eq!(msh[8], "ACK^A01^ACK");
        assert_ne!(msh[9], "MSG001", "the ACK has its own control id");
        assert_eq!(msh[9].len(), 20);
        assert_eq!(
            &msh[10..12],
            ["T", "2.4"],
            "a test message is acknowledged as a test"
        );
        let msa = fields(&a, "MSA");
        assert_eq!(&msa[..2], ["AA", "MSG001"]);
    }

    #[test]
    fn defaults_only_what_the_inbound_left_empty() {
        let i = inbound("MSH|^~\\&|EMR||||20260523120000||ADT^A01|MSG001||");
        let msh = fields(&ack_to(&i, AckCode::Accept, None), "MSH");
        assert_eq!(msh[2], "PAS");
        assert_eq!(&msh[10..12], ["P", "2.5"]);
    }

    #[test]
    fn a_reason_with_delimiters_and_line_breaks_cannot_change_the_structure() {
        let i = inbound("MSH|^~\\&|EMR|F|PAS|F|20260523120000||ADT^A01|MSG001|P|2.5");
        let reason = "bad | field ^ comp ~ rep \\ esc & sub\r\nMSH|^~\\&|FORGED";
        let a = ack_to(&i, AckCode::AppError, Some(reason));
        let msg = parse_message(&a).unwrap();
        assert_eq!(msg.segments.len(), 2, "still one MSH and one MSA: {a:?}");
        let msa = msg.segment("MSA").unwrap();
        assert_eq!(msa.raw(1), "AE");
        assert_eq!(msa.raw(2), "MSG001");
        assert_eq!(msa.raw(4), "", "nothing spilled into a further field");
        let (code, text) = ack_code_and_text(&a).unwrap();
        assert_eq!(code, AckCode::AppError);
        assert_eq!(
            text.unwrap(),
            "bad | field ^ comp ~ rep \\ esc & sub  MSH|^~\\&|FORGED"
        );
    }

    #[test]
    fn a_long_reason_is_cut_to_the_field_length() {
        let i = inbound("MSH|^~\\&|EMR|F|PAS|F|20260523120000||ADT^A01|MSG001|P|2.5");
        let a = ack_to(&i, AckCode::AppError, Some(&"x".repeat(500)));
        let (_, text) = ack_code_and_text(&a).unwrap();
        assert_eq!(text.unwrap().chars().count(), MSA3_MAX);
    }

    #[test]
    fn the_bare_form_still_names_both_ends() {
        let s = ack("PAS", "EMR", "MSG001", AckCode::Accept, None);
        assert!(s.starts_with("MSH|^~\\&|PAS||EMR||"), "{s}");
        assert!(s.contains("|P|2.5\r"));
        assert!(s.contains("MSA|AA|MSG001\r"));
        let s = ack(
            "PAS",
            "EMR",
            "MSG002",
            AckCode::AppError,
            Some("PID-5 missing"),
        );
        assert!(s.contains("MSA|AE|MSG002|PID-5 missing\r"));
    }
}

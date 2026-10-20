//! HL7 v2 message parser (pipe-delimited, standard delimiters only).

use super::Message;
use crate::{Error, Result};

/// Parse a v2 message string into a structured [`Message`].
///
/// The structure is read by the `er7` crate, which accepts `\r`, `\n` and `\r\n` as segment terminators.
/// This layer adds two rules of its own: the first segment must be `MSH`, and the delimiters must be the
/// standard `|^~\&`. Anything else is an [`Error::Validation`]. Each field keeps its wire text (still escaped),
/// and for `MSH`, field 1 is the separator and field 2 the encoding characters, as HL7 numbers them.
pub fn parse_message(raw: &str) -> Result<Message> {
    let parsed = er7::parse(raw).map_err(|e| Error::validation(format!("HL7v2: {e}")))?;
    let first = parsed.segments.first().map(|s| s.name.as_str());
    if first != Some("MSH") {
        return Err(Error::validation(format!(
            "HL7v2: first segment must be MSH, got {:?}",
            first.unwrap_or_default()
        )));
    }
    if parsed.separators != er7::Separators::default() {
        return Err(Error::validation(format!(
            "HL7v2: encoding characters must be \"^~\\&\", got {:?}",
            parsed.separators.encoding_characters()
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::SegmentRead;

    const A01: &str = "MSH|^~\\&|SENDAPP|FAC|RECVAPP|FAC|20260523120000||ADT^A01|MSG001|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001\r";

    #[test]
    fn test_parse_segment_names() {
        let m = parse_message(A01).expect("parse");
        let names: Vec<&str> = m.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["MSH", "EVN", "PID", "PV1"]);
    }

    #[test]
    fn test_parse_msh_keeps_encoding_in_field_2() {
        let m = parse_message(A01).expect("parse");
        let msh = m.segment("MSH").expect("MSH");
        // MSH-2 is the encoding characters.
        assert_eq!(msh.raw(2), "^~\\&");
        // MSH-3 is the sending app.
        assert_eq!(msh.raw(3), "SENDAPP");
        // MSH-9 is the message type.
        assert_eq!(msh.raw(9), "ADT^A01");
        assert_eq!(msh.component_text(9, 1), "ADT");
        assert_eq!(msh.component_text(9, 2), "A01");
    }

    #[test]
    fn test_parse_pid_components() {
        let m = parse_message(A01).expect("parse");
        let pid = m.segment("PID").expect("PID");
        assert_eq!(pid.raw(1), "1");
        assert_eq!(pid.raw(3), "MRN-001^^^FAC^MR");
        assert_eq!(pid.component_text(3, 1), "MRN-001");
        assert_eq!(pid.component_text(3, 5), "MR");
        assert_eq!(pid.raw(5), "Doe^Jane^Marie");
        assert_eq!(pid.component_text(5, 1), "Doe");
        assert_eq!(pid.component_text(5, 2), "Jane");
        assert_eq!(pid.component_text(5, 3), "Marie");
        assert_eq!(pid.raw(7), "19900115");
        assert_eq!(pid.raw(8), "F");
        assert_eq!(pid.component_text(11, 3), "Springfield");
        assert_eq!(pid.raw(13), "(555)555-0100");
    }

    #[test]
    fn test_parse_pv1_class_and_location() {
        let m = parse_message(A01).expect("parse");
        let pv1 = m.segment("PV1").expect("PV1");
        assert_eq!(pv1.raw(2), "I");
        assert_eq!(pv1.component_text(3, 1), "WARD1");
        assert_eq!(pv1.component_text(3, 2), "ROOM1");
        assert_eq!(pv1.component_text(3, 3), "BED1");
    }

    #[test]
    fn test_parse_accepts_lf_line_endings() {
        let lf = A01.replace('\r', "\n");
        let m = parse_message(&lf).expect("parse LF");
        assert_eq!(m.segments.len(), 4);
    }

    #[test]
    fn test_parse_rejects_missing_msh() {
        let bad = "PID|1||X||Doe^Jane||19900115|F\r";
        let err = parse_message(bad).expect_err("must reject");
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn test_parse_rejects_non_standard_delimiters() {
        let bad = "MSH#$%@&|X|Y\r";
        assert!(matches!(parse_message(bad), Err(Error::Validation(_))));
    }
}

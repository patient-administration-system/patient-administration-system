//! HL7 v2 message encoder (pipe-delimited, standard delimiters only).

use super::Message;
use er7::RenderOptions;

/// Encode a [`Message`] to a wire string: segments joined by `\r`, with a trailing `\r` (the HL7 convention).
/// Rendering is the `er7` crate's, which keeps what was parsed byte for byte.
pub fn encode_message(m: &Message) -> String {
    m.to_er7_with(RenderOptions {
        trailing_terminator: true,
        ..RenderOptions::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::parse_message;

    const A01: &str = "MSH|^~\\&|SENDAPP|FAC|RECVAPP|FAC|20260523120000||ADT^A01|MSG001|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001\r";

    #[test]
    fn test_encode_roundtrip_a01() {
        let parsed = parse_message(A01).expect("parse");
        let out = encode_message(&parsed);
        assert_eq!(out, A01, "round-trip must be byte-identical");
    }

    #[test]
    fn test_encode_empty_msh_fields_preserve_pipes() {
        let parsed = parse_message(A01).expect("parse");
        let out = encode_message(&parsed);
        // MSH-8 (security) is empty in the source; the leading pipes must
        // survive so field 9 stays at position 9.
        assert!(out.contains("|20260523120000||ADT^A01|"));
    }
}

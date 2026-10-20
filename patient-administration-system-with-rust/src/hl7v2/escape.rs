//! HL7 v2 escape sequences.
//!
//! v2 reserves five characters as field/component/repetition/subcomponent
//! separators and the escape character itself. When a *value* needs to
//! contain one of those characters literally, the sender escapes it with the
//! sequence `\X\` where `X` identifies which delimiter is being escaped:
//!
//! | Escape | Means literal | ASCII |
//! |--------|---------------|-------|
//! | `\F\`  | field sep     | `|`   |
//! | `\S\`  | component sep | `^`   |
//! | `\T\`  | subcomp sep   | `&`   |
//! | `\R\`  | repetition    | `~`   |
//! | `\E\`  | escape char   | `\`   |
//!
//! This module provides the two functions that should be applied at the
//! *domain ↔ wire* boundary:
//!
//! - [`escape_value`] when serializing a domain string into a v2 field/
//!   component value (encoder side).
//! - [`unescape_value`] when reading a v2 field/component value back into a
//!   domain string (parser/mapping side).
//!
//! These deliberately do NOT touch the raw segment bytes or the
//! `Segment::fields` storage in [`crate::hl7v2::Segment`] — that storage is
//! still the literal wire bytes between delimiters. Callers in
//! [`crate::hl7v2::mapping`] apply the escapes when crossing the boundary.

use er7::Separators;

/// Escape every reserved HL7 v2 delimiter in `value`, and a carriage return or line feed (which would
/// otherwise end the segment), so the value cannot change the structure of the message. Done by the `er7` crate.
pub fn escape_value(value: &str) -> String {
    er7::escape::escape(value, &Separators::default()).into_owned()
}

/// Inverse of [`escape_value`]. Sequences that stand for a character decode (`\F\ \S\ \T\ \R\ \E\` and hex
/// `\Xnn\`); every other sequence (`\H\`, `\N\`, an unterminated one) is kept as written so vendor extensions are
/// not lost. Done by the `er7` crate.
pub fn unescape_value(value: &str) -> String {
    er7::escape::unescape(value, &Separators::default()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_protects_the_segment_from_line_breaks() {
        let e = escape_value("line\r\nnext");
        assert!(!e.contains('\r') && !e.contains('\n'));
        assert_eq!(unescape_value(&e), "line\r\nnext");
    }

    #[test]
    fn test_escape_preserves_normal_strings() {
        assert_eq!(escape_value("Doe"), "Doe");
        assert_eq!(escape_value("Jane Marie"), "Jane Marie");
    }

    #[test]
    fn test_escape_each_delimiter() {
        assert_eq!(escape_value("a|b"), "a\\F\\b");
        assert_eq!(escape_value("a^b"), "a\\S\\b");
        assert_eq!(escape_value("a&b"), "a\\T\\b");
        assert_eq!(escape_value("a~b"), "a\\R\\b");
        assert_eq!(escape_value("a\\b"), "a\\E\\b");
    }

    #[test]
    fn test_escape_handles_multiple_delimiters_in_one_value() {
        assert_eq!(escape_value("O^Brien-Jones"), "O\\S\\Brien-Jones");
        assert_eq!(escape_value("a|b^c&d"), "a\\F\\b\\S\\c\\T\\d");
    }

    #[test]
    fn test_unescape_roundtrip() {
        let cases = [
            "Doe",
            "O^Brien",
            "a|b",
            "a&b",
            "a~b",
            "back\\slash",
            "all|of^the~separators&plus\\escape",
        ];
        for c in cases {
            let escaped = escape_value(c);
            let unescaped = unescape_value(&escaped);
            assert_eq!(unescaped, c, "round-trip failed for {c:?}");
        }
    }

    #[test]
    fn test_unescape_unknown_sequence_passes_through() {
        // Vendor extensions like \H\ / \N\ / hex \X41\ should not be lost.
        assert_eq!(unescape_value("text\\H\\more"), "text\\H\\more");
    }

    #[test]
    fn test_unescape_does_not_match_lone_backslash() {
        // A backslash that's not part of a complete \X\ sequence is literal.
        assert_eq!(unescape_value("path\\to\\file"), "path\\to\\file");
    }

    #[test]
    fn test_escape_does_not_double_escape_existing_backslash() {
        // Input is "a\F\b" (with a literal backslash). Escaping should treat
        // each backslash as an escape-char to escape, NOT recognize the
        // existing "\F\" as a delimiter token.
        let escaped = escape_value("a\\F\\b");
        assert_eq!(escaped, "a\\E\\F\\E\\b");
        let back = unescape_value(&escaped);
        assert_eq!(back, "a\\F\\b");
    }
}

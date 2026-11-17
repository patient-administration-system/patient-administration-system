//! HL7 v2 for logs, with the personal data taken out (NFR-33, HUB-5).
//!
//! Only the header says what a message is and where it came from, so only the header is kept: MSH-2 to MSH-7
//! and MSH-9 to MSH-12 (encoding characters, sending and receiving application and facility, time, type,
//! control id, processing id, version). MSH-8 (security) and every field of every other segment that holds
//! anything becomes `***`; an empty field stays empty, so the message keeps its shape and a reader can still
//! see which fields were sent. Segment names are kept. Text that does not start with `MSH`, `BHS` or `FHS` is
//! not taken apart: it is replaced whole with its length.
//!
//! The control id is kept because it is how a sender's log and ours meet. Some senders build it from a patient
//! identifier; a deployment that has one should log less (only `Key::short` in `dedupe` is safe everywhere).
//!
//! This works on raw text, never panics, and reads the field separator as the fourth *character*, so
//! multi-byte text cannot split it.

/// The redacted text of `message`.
pub fn redact(message: &str) -> String {
    let trimmed = message.trim_start();
    let header = ["MSH", "BHS", "FHS"].iter().any(|h| trimmed.starts_with(h));
    let Some(sep) = trimmed.chars().nth(3).filter(|_| header) else {
        return format!("<not HL7 v2: {} bytes>", message.len());
    };
    if sep.is_alphanumeric() || sep == '\r' || sep == '\n' {
        return format!("<not HL7 v2: {} bytes>", message.len());
    }
    let mut out = String::with_capacity(trimmed.len());
    for (i, segment) in trimmed.split(['\r', '\n']).enumerate() {
        if segment.is_empty() {
            continue;
        }
        if i > 0 && !out.is_empty() {
            out.push('\r');
        }
        out.push_str(&redact_segment(segment, sep));
    }
    out
}

fn redact_segment(segment: &str, sep: char) -> String {
    let mut fields = segment.split(sep);
    let name = fields.next().unwrap_or_default();
    // A segment name is three letters or digits; anything else is not kept either.
    let name_ok = name.len() == 3 && name.chars().all(|c| c.is_ascii_alphanumeric());
    let mut out = String::from(if name_ok { name } else { "***" });
    let is_header = matches!(name, "MSH" | "BHS" | "FHS");
    // For MSH, the field separator itself is MSH-1, so the first field after the name is MSH-2.
    for (offset, field) in fields.enumerate() {
        let number = offset + 2;
        out.push(sep);
        let keep =
            is_header && (2..=7).contains(&number) || is_header && (9..=12).contains(&number);
        if field.is_empty() || keep {
            out.push_str(field);
        } else {
            out.push_str("***");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEEDLE: &str = "NEEDLE7391";

    #[test]
    fn keeps_the_header_and_masks_the_rest_keeping_the_shape() {
        let m = format!(
            "MSH|^~\\&|EMR|WARDS|PAS|HOSP|20261117120000|SECRET|ADT^A01|MSG-1|P|2.5\r\
             PID|1||{NEEDLE}^^^HOSP^MR||Lovelace^{NEEDLE}||19900115|F\rPV1|1|I|W^R^B1"
        );
        let r = redact(&m);
        assert_eq!(
            r,
            "MSH|^~\\&|EMR|WARDS|PAS|HOSP|20261117120000|***|ADT^A01|MSG-1|P|2.5\r\
             PID|***||***||***||***|***\rPV1|***|***|***"
        );
        assert!(!r.contains(NEEDLE));
    }

    #[test]
    fn anything_that_is_not_hl7_is_replaced_whole() {
        for text in [NEEDLE, "", "GET / HTTP/1.1", "MSHX", "MSH", "MS"] {
            let r = redact(text);
            assert!(r.starts_with("<not HL7 v2: "), "{text:?} → {r}");
            assert!(!r.contains(NEEDLE));
        }
    }

    #[test]
    fn multibyte_text_does_not_panic_or_leak() {
        // The hub this idea came from found its redactor panicked on MSH followed by a non-ASCII character.
        for text in [
            "MSHé",
            "MSH€|x",
            "MSH|^~\\&|é\rPID|€€€|NEEDLE7391",
            "BHS|^~\\&|\u{1F600}",
        ] {
            let r = redact(text);
            assert!(!r.contains(NEEDLE), "{text:?} → {r}");
        }
    }

    #[test]
    fn a_batch_header_is_kept_like_msh() {
        let r = redact("BHS|^~\\&|EMR|WARDS|PAS|HOSP|20261117||||B-1\rMSH|^~\\&|EMR\rPID|1||X");
        assert!(
            r.starts_with("BHS|^~\\&|EMR|WARDS|PAS|HOSP|20261117"),
            "{r}"
        );
        assert!(r.ends_with("PID|***||***"), "{r}");
    }
}

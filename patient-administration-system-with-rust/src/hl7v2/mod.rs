//! HL7 v2 message parsing, encoding, and mapping to PAS domain types. The message model, parsing,
//! escaping and rendering are the `er7` crate's; this module adds the PAS mappings and the transport
//! around it (MLLP, batches, the listener).
//!
//! Pipe-delimited messages with the standard delimiter set `|^~\&`. Supported segments: `MSH`, `EVN`, `PID`, `PV1`. Supported
//! message types for round-trip mapping: `ADT^A01` (admit), `ADT^A03`
//! (discharge), `ADT^A28` (add person info). Other message types parse to
//! the generic [`Message`] structure but are not mapped to domain objects.
//!
//! Wire format primer:
//!
//! ```text
//! MSH|^~\&|SENDING_APP|FAC|RECV_APP|FAC|20260523120000||ADT^A01|MSG001|P|2.5
//! EVN|A01|20260523120000
//! PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100
//! PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001
//! ```
//!
//! - Segments separated by `\r` (the HL7 convention; this module also accepts
//!   `\n` and `\r\n` on the read path).
//! - Fields separated by `|`.
//! - Components separated by `^`.
//! - Subcomponents separated by `&`.
//! - Repetitions separated by `~`.
//!
//! Field numbering follows the HL7 convention: `PID-5` is the *fifth* field of
//! the segment, which means `segment.raw(5)` returns it. For MSH, field 1
//! is the field separator itself; field 2 is the encoding characters
//! (`^~\&`).

pub mod ack;
pub mod batch;
pub mod encode;
pub mod encoder;
pub mod escape;
pub mod listener;
pub mod mllp;
pub mod parser;
pub mod pid_translation;

pub use ack::{AckCode, ack};
pub use batch::{Batch, MAX_BATCH_MESSAGES, encode_batch_ack, looks_like_batch, parse_batch};
pub use encode::{EncodeContext, encode_adt, encode_siu, fmt_ts};
pub use encoder::encode_message;
pub use escape::{escape_value, unescape_value};
pub use listener::MllpServer;
pub use parser::parse_message;

// NOTE: there is no general `mapping` module. PID/STF/MFE shape conversion happens in `pid_translation` and
// in the handlers, which translate between the upstream `Person` / `Worker` JSON and the HL7 v2 wire format.

/// The message model is the `er7` crate's.
pub use er7::{Message, Segment, Separators};

/// Read a segment's fields as the wire text, the way the mapping code wants them (still escaped, with its
/// `^`, `~` and `&`). Index 1 is the first field, as HL7 numbers them; anything absent is `""`.
pub trait SegmentRead {
    /// The whole text of field `n`.
    fn raw(&self, n: usize) -> String;
    /// The text of component `c` of field `n`.
    fn component_text(&self, n: usize, c: usize) -> String;
}

impl SegmentRead for Segment {
    fn raw(&self, n: usize) -> String {
        let separators = Separators::default();
        n.checked_sub(1)
            .and_then(|i| self.fields.get(i))
            .map(|f| f.to_er7(&separators))
            .unwrap_or_default()
    }

    fn component_text(&self, n: usize, c: usize) -> String {
        let separators = Separators::default();
        self.component(n, c)
            .map(|x| x.to_er7(&separators))
            .unwrap_or_default()
    }
}

/// Build a segment from its fields' wire text. For `MSH` pass the fields in HL7 order, so the first is the
/// field separator and the second the encoding characters.
pub fn segment(name: &str, fields: Vec<String>) -> Segment {
    let separators = Separators::default();
    let mut text = name.to_string();
    if name == "MSH" && fields.len() >= 2 {
        text.push_str(&fields[0]);
        text.push_str(&fields[1]);
        for f in &fields[2..] {
            text.push('|');
            text.push_str(f);
        }
    } else {
        for f in &fields {
            text.push('|');
            text.push_str(f);
        }
    }
    er7::parse_with(&text, separators)
        .segments
        .into_iter()
        .next()
        .unwrap_or(Segment {
            name: name.to_string(),
            fields: Vec::new(),
        })
}

/// A message of the given segments with the standard delimiters.
pub fn message(segments: Vec<Segment>) -> Message {
    Message {
        separators: Separators::default(),
        segments,
    }
}

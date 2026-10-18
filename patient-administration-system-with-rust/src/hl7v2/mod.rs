//! HL7 v2 message parsing, encoding, and mapping to PAS domain types.
//!
//! v0.3 first cut: pipe-delimited ADT messages with the standard delimiter
//! set `|^~\&`. Supported segments: `MSH`, `EVN`, `PID`, `PV1`. Supported
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
//! the segment, which means `segment.field(5)` returns it. For MSH, field 1
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

// NOTE: `mapping` module deleted in v0.44.0 Option C. Every PID/STF/MFE
// shape conversion now happens inside the proxy handlers in Phase 3,
// which translate between the upstream `Person` / `Worker` JSON and the
// HL7 v2 wire format directly. ADT/SIU/DFT/MFN encoders are rebuilt as
// proxy-handler-local helpers in Phase 3.

use serde::{Deserialize, Serialize};

/// Standard HL7 delimiter set. Custom delimiters are out of scope — every
/// message this module produces uses `|^~\&`.
pub const FIELD_SEP: char = '|';
pub const COMPONENT_SEP: char = '^';
pub const REPETITION_SEP: char = '~';
pub const ESCAPE_CHAR: char = '\\';
pub const SUBCOMPONENT_SEP: char = '&';

/// One parsed HL7 v2 segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Three-letter segment name, e.g. `"MSH"`, `"PID"`.
    pub name: String,
    /// Raw field strings, including any embedded `^` / `~` / `&`. For the
    /// `MSH` segment, `fields[0]` is the field separator itself (`"|"`) and
    /// `fields[1]` is the encoding characters (`"^~\&"`), so `field(n)`
    /// matches HL7's 1-indexed numbering for every `n`.
    pub fields: Vec<String>,
}

impl Segment {
    /// Read field N (1-indexed, HL7-style). Returns `""` if the field is
    /// missing or beyond the end of the segment.
    pub fn field(&self, n: usize) -> &str {
        if n == 0 {
            return "";
        }
        // Field 1 = fields[0] (the segment-name field is dropped on parse).
        self.fields.get(n - 1).map(String::as_str).unwrap_or("")
    }

    /// Read component C (1-indexed) of field N (1-indexed). Returns `""` if
    /// either index is out of range.
    pub fn component(&self, field_n: usize, component_c: usize) -> &str {
        if component_c == 0 {
            return "";
        }
        let f = self.field(field_n);
        f.split(COMPONENT_SEP).nth(component_c - 1).unwrap_or("")
    }
}

/// A parsed HL7 v2 message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub segments: Vec<Segment>,
}

impl Message {
    /// Find the first segment with the given three-letter name.
    pub fn segment(&self, name: &str) -> Option<&Segment> {
        self.segments.iter().find(|s| s.name == name)
    }

    /// Iterator over every segment with the given name.
    pub fn all_segments<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Segment> + 'a {
        self.segments.iter().filter(move |s| s.name == name)
    }
}

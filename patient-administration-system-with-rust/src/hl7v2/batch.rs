//! HL7 v2 batch envelope (FHS / BHS / BTS / FTS).
//!
//! The HL7 v2 batch protocol wraps many messages into one transmission for
//! bulk send (end-of-day batches, backfill, system migrations). Two
//! envelopes:
//!
//! ```text
//! FHS|^~\&|...           ← file header   (optional, outer wrapper)
//!   BHS|^~\&|...         ← batch header  (required if there is a batch)
//!     MSH|^~\&|...
//!     [segments]
//!     MSH|^~\&|...
//!     [segments]
//!     ...
//!   BTS|<msg_count>      ← batch trailer (optional; count is informational)
//! FTS|<batch_count>      ← file trailer  (optional)
//! ```
//!
//! v0.6 first cut: one batch per request. A request may carry the file
//! envelope `FHS`/`FTS` around a single `BHS`/`BTS` batch (the most common
//! shape in the wild) or just a bare `BHS`/`BTS` batch. Multi-batch files
//! are *not* supported — every `BHS` after the first returns
//! `Error::Validation`.

use super::{Message, Segment, Separators};
use crate::{Error, Result};

/// Hard cap on how many `MSH` messages one batch may contain. Anything
/// larger returns `Error::Validation` so the batch endpoint can return a
/// single `AR` rather than spending unbounded effort.
pub const MAX_BATCH_MESSAGES: usize = 1000;

/// One parsed HL7 v2 batch transmission. `fhs`/`fts` are present only when
/// the sender wrapped the batch in a file envelope. `bhs`/`bts` are
/// present whenever the sender used the batch envelope (every real batch
/// does, but the parser will also accept a bare list of `MSH` messages
/// for convenience and surface that as `bhs = None`, `bts = None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub fhs: Option<Segment>,
    pub bhs: Option<Segment>,
    pub messages: Vec<Message>,
    pub bts: Option<Segment>,
    pub fts: Option<Segment>,
}

/// Parse one batch transmission. The segments are read by the `er7` crate; this walks them, recognising
/// `FHS`/`BHS`/`MSH`/`BTS`/`FTS`. Everything else is a body segment that attaches to the current `MSH`
/// message, and each further `MSH` starts the next message.
///
/// Errors:
/// - empty input or no `MSH`/`BHS`/`FHS` → `Error::Validation`.
/// - non-standard delimiters → `Error::Validation`.
/// - more than one `BHS` (multi-batch files) → `Error::Validation`.
/// - more than `MAX_BATCH_MESSAGES` messages → `Error::Validation`.
pub fn parse_batch(raw: &str) -> Result<Batch> {
    let parsed = er7::parse(raw).map_err(|e| Error::validation(format!("HL7v2 batch: {e}")))?;
    if parsed.separators != Separators::default() {
        return Err(Error::validation(format!(
            "HL7v2 batch: encoding characters must be \"^~\\&\", got {:?}",
            parsed.separators.encoding_characters()
        )));
    }

    let mut fhs = None;
    let mut bhs = None;
    let mut bts = None;
    let mut fts = None;
    let mut messages: Vec<Message> = Vec::new();
    // Segments of the message being read. Flushed at the next MSH, BTS, FTS, BHS or the end.
    let mut current: Option<Vec<Segment>> = None;
    let flush = |current: &mut Option<Vec<Segment>>, messages: &mut Vec<Message>| {
        if let Some(segments) = current.take() {
            messages.push(Message {
                separators: Separators::default(),
                segments,
            });
        }
    };

    for seg in parsed.segments {
        match seg.name.as_str() {
            "FHS" => {
                if fhs.is_some() {
                    return Err(Error::validation(
                        "HL7v2 batch: more than one FHS segment is not supported",
                    ));
                }
                flush(&mut current, &mut messages);
                fhs = Some(seg);
            }
            "BHS" => {
                if bhs.is_some() {
                    return Err(Error::validation(
                        "HL7v2 batch: more than one BHS in a single transmission is not supported \
                         (multi-batch files not implemented)",
                    ));
                }
                flush(&mut current, &mut messages);
                bhs = Some(seg);
            }
            "MSH" => {
                flush(&mut current, &mut messages);
                if messages.len() > MAX_BATCH_MESSAGES {
                    return Err(Error::validation(format!(
                        "HL7v2 batch: more than {MAX_BATCH_MESSAGES} messages",
                    )));
                }
                current = Some(vec![seg]);
            }
            "BTS" => {
                flush(&mut current, &mut messages);
                bts = Some(seg);
            }
            "FTS" => {
                flush(&mut current, &mut messages);
                fts = Some(seg);
            }
            name => match current.as_mut() {
                Some(segs) => segs.push(seg),
                None => {
                    return Err(Error::validation(format!(
                        "HL7v2 batch: segment {name:?} appears before any MSH"
                    )));
                }
            },
        }
    }
    flush(&mut current, &mut messages);
    if messages.len() > MAX_BATCH_MESSAGES {
        return Err(Error::validation(format!(
            "HL7v2 batch: more than {MAX_BATCH_MESSAGES} messages",
        )));
    }
    if messages.is_empty() && bhs.is_none() && fhs.is_none() {
        return Err(Error::validation(
            "HL7v2 batch: input must contain at least one MSH, BHS, or FHS segment",
        ));
    }
    Ok(Batch {
        fhs,
        bhs,
        messages,
        bts,
        fts,
    })
}

/// Build a batch ACK envelope. Wraps the per-message ACK bodies inside
/// one `BHS` / `BTS` pair so the sender can read each MSA on the same
/// connection.
///
/// The shape matches what most HL7 v2 receivers emit:
///
/// ```text
/// BHS|^~\&|PAS|FAC|<sender>|FAC|<now>||<batch_ctl_id>|P|2.5
/// <ack-1>
/// <ack-2>
/// ...
/// BTS|<n>
/// ```
///
/// Each per-message ACK is already a complete `MSH ... MSA` block produced
/// by [`crate::hl7v2::ack::ack`], so the function just stitches them
/// together with the envelope.
pub fn encode_batch_ack(
    sending_app: &str,
    receiving_app: &str,
    batch_control_id: &str,
    per_message_acks: &[String],
) -> String {
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let mut out = String::with_capacity(
        256 + per_message_acks
            .iter()
            .map(std::string::String::len)
            .sum::<usize>(),
    );
    out.push_str(&format!(
        "BHS|^~\\&|{sending_app}|FAC|{receiving_app}|FAC|{now}||{batch_control_id}|P|2.5\r"
    ));
    for ack in per_message_acks {
        // Each ack ends with `\r` already; preserve as-is so per-message MSH
        // and MSA segments are individually terminated.
        out.push_str(ack);
    }
    out.push_str(&format!("BTS|{}\r", per_message_acks.len()));
    out
}

/// Returns `true` if `payload` looks like a batch (first segment is `FHS`
/// or `BHS`). The MLLP listener uses this to pick the right route.
pub fn looks_like_batch(payload: &str) -> bool {
    let trimmed = payload.trim_start();
    trimmed.starts_with("FHS|") || trimmed.starts_with("BHS|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::AckCode;
    use crate::hl7v2::SegmentRead;
    use crate::hl7v2::ack;

    const A28: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG-1|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||MRN-1^^^FAC^MR||Smith^John||19800101|M\r";

    const A28_B: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120100||ADT^A28|MSG-2|P|2.5\r\
EVN|A28|20260523120100\r\
PID|1||MRN-2^^^FAC^MR||Jones^Mary||19850515|F\r";

    fn wrap_batch(messages: &[&str]) -> String {
        let mut s = String::from("BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||BATCH-001|P|2.5\r");
        for m in messages {
            s.push_str(m);
        }
        s.push_str(&format!("BTS|{}\r", messages.len()));
        s
    }

    #[test]
    fn test_parse_batch_with_two_messages() {
        let body = wrap_batch(&[A28, A28_B]);
        let b = parse_batch(&body).expect("parse");
        assert!(b.bhs.is_some(), "BHS must be parsed");
        assert!(b.bts.is_some(), "BTS must be parsed");
        assert_eq!(b.messages.len(), 2);
        assert_eq!(b.messages[0].segments[0].name, "MSH");
        assert_eq!(
            b.messages[0].segment("PID").unwrap().component_text(3, 1),
            "MRN-1"
        );
        assert_eq!(
            b.messages[1].segment("PID").unwrap().component_text(3, 1),
            "MRN-2"
        );
        let bts = b.bts.as_ref().unwrap();
        assert_eq!(bts.raw(1), "2");
    }

    #[test]
    fn test_parse_batch_accepts_file_envelope() {
        let body = format!(
            "FHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||FILE-001|P|2.5\r\
{}FTS|1\r",
            wrap_batch(&[A28])
        );
        let b = parse_batch(&body).expect("parse FHS+BHS+FTS");
        assert!(b.fhs.is_some());
        assert!(b.bhs.is_some());
        assert!(b.bts.is_some());
        assert!(b.fts.is_some());
        assert_eq!(b.messages.len(), 1);
    }

    #[test]
    fn test_parse_batch_accepts_bare_messages_without_envelope() {
        // Just an MSH list without BHS — accept it (some senders are lazy).
        let body = format!("{A28}{A28_B}");
        let b = parse_batch(&body).expect("parse bare MSH list");
        assert!(b.bhs.is_none());
        assert_eq!(b.messages.len(), 2);
    }

    #[test]
    fn test_parse_batch_rejects_empty_input() {
        assert!(matches!(parse_batch(""), Err(Error::Validation(_))));
        assert!(matches!(parse_batch("\r\n\r\n"), Err(Error::Validation(_))));
    }

    #[test]
    fn test_parse_batch_rejects_body_segment_before_any_msh() {
        let body = "BHS|^~\\&|E|F|R|F|20260523120000||B-1|P|2.5\r\
PID|1||MRN-X^^^FAC^MR||Nobody^A||19800101|F\r\
BTS|0\r";
        assert!(matches!(parse_batch(body), Err(Error::Validation(_))));
    }

    #[test]
    fn test_parse_batch_rejects_second_bhs() {
        let body = format!(
            "{}BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120100||BATCH-002|P|2.5\r{A28}BTS|1\r",
            wrap_batch(&[A28]),
        );
        let err = parse_batch(&body).expect_err("two BHS must fail");
        match err {
            Error::Validation(msg) => assert!(msg.contains("BHS"), "diag: {msg}"),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_batch_rejects_oversize_input() {
        let mut body = String::from("BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||B-1|P|2.5\r");
        for _ in 0..(MAX_BATCH_MESSAGES + 1) {
            body.push_str(A28);
        }
        body.push_str("BTS|0\r");
        let err = parse_batch(&body).expect_err("over limit must fail");
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn test_encode_batch_ack_wraps_per_message_acks() {
        let a1 = ack("PAS", "EMR", "MSG-1", AckCode::Accept, None);
        let a2 = ack("PAS", "EMR", "MSG-2", AckCode::AppError, Some("bad PID"));
        let out = encode_batch_ack("PAS", "EMR", "ACK-BATCH-1", &[a1.clone(), a2.clone()]);
        assert!(out.starts_with("BHS|^~\\&|PAS|FAC|EMR|FAC|"));
        assert!(out.contains("|ACK-BATCH-1|P|2.5\r"));
        assert!(out.contains(&a1));
        assert!(out.contains(&a2));
        assert!(out.contains("MSA|AA|MSG-1"));
        assert!(out.contains("MSA|AE|MSG-2|bad PID"));
        assert!(out.ends_with("BTS|2\r"));
    }

    #[test]
    fn test_looks_like_batch_detects_bhs_and_fhs() {
        assert!(looks_like_batch("BHS|^~\\&|E|F|R|F|datetime||X|P|2.5\r"));
        assert!(looks_like_batch("  FHS|^~\\&|E|F|R|F|datetime||X|P|2.5\r"));
        assert!(!looks_like_batch(
            "MSH|^~\\&|E|F|R|F|datetime||ADT^A01|X|P|2.5\r"
        ));
        assert!(!looks_like_batch("garbage"));
        assert!(!looks_like_batch(""));
    }

    #[test]
    fn test_multibyte_text_where_a_segment_name_should_be_is_an_error_not_a_panic() {
        // Found by the chaos monkey: slicing the first three bytes split a character.
        for raw in [
            "💥🔥ñ\rMSH|^~\\&|",
            "FHS|^~\\&|\r💥",
            "ñ",
            "\u{202e}rtl",
            "BHS|^~\\&|\rBT💥",
        ] {
            let _ = parse_batch(raw); // must return, Ok or Err
        }
        assert!(parse_batch("💥🔥ñ").is_err());
    }
}

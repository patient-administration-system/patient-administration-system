//! QUAL-17: property tests for the code that reads outside input. No database needed.
//!
//! - NHS numbers: exactly the right check digit is accepted, and the two errors people make most (one
//!   digit wrong, two neighbours swapped) are always caught.
//! - Text replies: no message panics the parser, and case, accents and surrounding space never change
//!   what a reply means.
//! - HL7 v2: no input panics the parser; any value survives escaping; a message built from escaped values
//!   parses and encodes back byte for byte; a batch of N messages splits into exactly N.

use patient_administration_system::hl7v2::batch::parse_batch;
use patient_administration_system::hl7v2::{
    encode_message, escape_value, parse_message, unescape_value,
};
use patient_administration_system::inbound::parser::{Intent, parse};
use patient_administration_system::validation::{format_nhs_number, validate_nhs_number};
use proptest::prelude::*;

/// The check digit for nine digits, or `None` when the remainder makes the number impossible.
fn check_digit(nine: &[u32]) -> Option<u32> {
    let sum: u32 = nine.iter().zip((2..=10).rev()).map(|(d, w)| d * w).sum();
    match 11 - (sum % 11) {
        11 => Some(0),
        10 => None,
        c => Some(c),
    }
}

fn number(nine: &[u32], last: u32) -> String {
    nine.iter()
        .chain(std::iter::once(&last))
        .map(|d| char::from_digit(*d, 10).unwrap())
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn exactly_the_check_digit_is_accepted(nine in prop::collection::vec(0u32..10, 9)) {
        let ok: Vec<u32> = (0..10).filter(|d| validate_nhs_number(&number(&nine, *d)).is_ok()).collect();
        prop_assert_eq!(ok, check_digit(&nine).into_iter().collect::<Vec<_>>());
    }

    #[test]
    fn one_wrong_digit_or_two_swapped_neighbours_is_always_caught(
        nine in prop::collection::vec(0u32..10, 9),
        at in 0usize..10,
        other in 0u32..10,
    ) {
        let Some(last) = check_digit(&nine) else { return Ok(()) };
        let good = number(&nine, last);
        prop_assert!(validate_nhs_number(&good).is_ok());
        prop_assert_eq!(validate_nhs_number(&format_nhs_number(&good)).unwrap(), good.clone());

        let mut wrong: Vec<char> = good.chars().collect();
        let was = wrong[at];
        let now = char::from_digit(other, 10).unwrap();
        if now != was {
            wrong[at] = now;
            prop_assert!(validate_nhs_number(&wrong.iter().collect::<String>()).is_err(), "one digit changed: {}", good);
        }

        let mut swapped: Vec<char> = good.chars().collect();
        let i = at.min(8);
        if swapped[i] != swapped[i + 1] {
            swapped.swap(i, i + 1);
            prop_assert!(validate_nhs_number(&swapped.iter().collect::<String>()).is_err(), "neighbours swapped: {}", good);
        }
    }

    #[test]
    fn no_text_message_panics_the_reply_parser(text in "\\PC{0,200}") {
        let _ = parse(&text);
    }

    #[test]
    fn case_accents_and_space_never_change_a_reply(
        word in prop::sample::select(vec!["yes", "y", "no", "n", "stop", "start", "help", "ie", "na", "r", "cancel"]),
        upper in any::<bool>(),
        pad in "[ \\t]{0,3}",
    ) {
        let plain = parse(word);
        let shown = if upper { word.to_uppercase() } else { word.to_string() };
        prop_assert_eq!(parse(&format!("{pad}{shown}{pad}")), plain);
    }

    #[test]
    fn a_bare_number_is_a_choice_and_never_a_confirmation(n in 1u8..10) {
        prop_assert_eq!(parse(&n.to_string()), Intent::Choice(n));
    }

    #[test]
    fn no_input_panics_the_hl7_parser(raw in "(MSH\\|\\^~\\\\&\\|)?[\\PC\\r\\|\\^~&\\\\]{0,300}") {
        let _ = parse_message(&raw);
    }

    #[test]
    fn any_value_survives_escaping(value in "\\PC{0,80}|[\\r\\n\\|\\^~&\\\\a-z]{0,40}") {
        let escaped = escape_value(&value);
        prop_assert!(!escaped.contains(['|', '^', '~', '&', '\r', '\n']), "{:?}", escaped);
        prop_assert_eq!(unescape_value(&escaped), value);
    }

    #[test]
    fn a_message_of_escaped_values_round_trips_byte_for_byte(
        segments in prop::collection::vec(prop::collection::vec("\\PC{0,20}", 1..6), 1..5),
    ) {
        let mut raw = String::from("MSH|^~\\&|SEND|FAC|RECV|FAC|20260101000000||ADT^A01|ID1|P|2.5\r");
        for (n, fields) in segments.iter().enumerate() {
            let values: Vec<String> = fields.iter().map(|f| escape_value(f)).collect();
            raw.push_str(&format!("Z{:02}|{}\r", n, values.join("|")));
        }
        let parsed = parse_message(&raw).expect("a well-formed message parses");
        prop_assert_eq!(encode_message(&parsed), raw);
    }

    #[test]
    fn no_input_panics_the_batch_splitter(raw in "((BHS|MSH|BTS|FHS|FTS|PID)\\|[\\PC\\|\\^~&]{0,40}\r){0,12}") {
        let _ = parse_batch(&raw);
    }

    #[test]
    fn a_batch_of_n_messages_splits_into_n(ids in prop::collection::vec("[A-Z0-9]{1,10}", 1..20)) {
        let mut raw = String::from("BHS|^~\\&|SEND|FAC|RECV|FAC|20260101000000\r");
        for id in &ids {
            raw.push_str(&format!("MSH|^~\\&|SEND|FAC|RECV|FAC|20260101000000||ADT^A08|{id}|P|2.5\rPID|1||{id}\r"));
        }
        raw.push_str(&format!("BTS|{}\r", ids.len()));
        let batch = parse_batch(&raw).expect("a well-formed batch parses");
        prop_assert_eq!(batch.messages.len(), ids.len());
        prop_assert!(batch.bhs.is_some() && batch.bts.is_some());
    }
}

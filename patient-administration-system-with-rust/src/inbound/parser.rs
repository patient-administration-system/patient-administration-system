//! Reads a patient's text reply (PILOT-4). Pure: no database, so every rule is unit-tested.
//!
//! A reply is understood only when the **whole** message is a keyword (optionally with a polite
//! word), because acting on a half-understood message (cancelling someone's appointment because they
//! wrote "no problem, see you there") is far worse than asking again. Anything else is `Unknown`,
//! and the patient is asked to reply with a keyword.
//!
//! English and Welsh are both understood whatever language the patient is registered with: people
//! reply in the language they are in the mood for. Matching ignores case, accents (so `cadarnhau` and
//! `cadarnhâu` agree), apostrophes and trailing punctuation.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Confirm,
    Cancel,
    Reschedule,
    /// Opt out of messages (PILOT-7 acts on this).
    Stop,
    Help,
    /// A bare number: the answer to an "which appointment?" question.
    Choice(u8),
    Unknown,
}

/// Fold a message to a comparable form: lower case, accents removed, only letters, digits and
/// single spaces kept.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ẁ' | 'ẃ' | 'ŵ' | 'ẅ' => 'w',
            'ỳ' | 'ý' | 'ŷ' | 'ÿ' => 'y',
            '\'' | '’' | '‘' | '`' | '´' => continue, // can't = cant
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

const CONFIRM: &[&str] = &[
    "y",
    "yes",
    "yep",
    "yeah",
    "ok",
    "okay",
    "confirm",
    "confirmed",
    "c",
    "see you there",
    "ie",
    "iawn",
    "ydw",
    "cadarnhau",
    "cadarnhaf",
    "cadarnhad",
];
const CANCEL: &[&str] = &[
    "n",
    "no",
    "cancel",
    "cancelled",
    "cant come",
    "cant make it",
    "cannot come",
    "cant attend",
    "na",
    "nac",
    "dim",
    "canslo",
    "ni allaf ddod",
    "ni allaf",
];
const RESCHEDULE: &[&str] = &[
    "r",
    "reschedule",
    "rebook",
    "change",
    "change it",
    "move",
    "another time",
    "different time",
    "newid",
    "aildrefnu",
    "symud",
    "amser arall",
];
const STOP: &[&str] = &[
    "stop",
    "stop all",
    "unsubscribe",
    "end",
    "quit",
    "opt out",
    "optout",
    "stopio",
    "diwedd",
    "dad danysgrifio",
];
const HELP: &[&str] = &["help", "info", "cymorth", "?"];
/// Words that may follow a keyword without changing its meaning.
const POLITE: &[&str] = &[
    "please",
    "thanks",
    "thank you",
    "ta",
    "plis",
    "os gwelwch yn dda",
    "diolch",
];

fn matches(set: &[&str], s: &str) -> bool {
    set.contains(&s)
}

/// Read one message.
pub fn parse(text: &str) -> Intent {
    let mut s = fold(text);
    // `?` folds to nothing, so check it before folding.
    if text.trim() == "?" {
        return Intent::Help;
    }
    // Strip one trailing polite word ("yes thanks", "cancel please").
    for p in POLITE {
        if let Some(rest) = s.strip_suffix(p)
            && rest.ends_with(' ')
        {
            s = rest.trim().to_string();
            break;
        }
    }
    if s.is_empty() {
        return Intent::Unknown;
    }
    if matches(CONFIRM, &s) {
        return Intent::Confirm;
    }
    if matches(CANCEL, &s) {
        return Intent::Cancel;
    }
    if matches(RESCHEDULE, &s) {
        return Intent::Reschedule;
    }
    if matches(STOP, &s) {
        return Intent::Stop;
    }
    if matches(HELP, &s) {
        return Intent::Help;
    }
    if s.len() == 1
        && let Some(d) = s.chars().next().and_then(|c| c.to_digit(10))
        && (1..=9).contains(&d)
    {
        return Intent::Choice(d as u8);
    }
    Intent::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(words: &[&str], want: Intent) {
        for w in words {
            assert_eq!(parse(w), want, "{w:?}");
        }
    }

    #[test]
    fn test_english_keywords_in_any_case_with_punctuation() {
        all(
            &[
                "Y",
                "yes",
                "YES!",
                " Yes. ",
                "OK",
                "Confirm",
                "see you there",
            ],
            Intent::Confirm,
        );
        all(
            &[
                "N",
                "No",
                "CANCEL",
                "Can't come",
                "can’t make it",
                "cant attend",
            ],
            Intent::Cancel,
        );
        all(
            &["R", "reschedule", "Change", "another time"],
            Intent::Reschedule,
        );
        all(
            &["STOP", "Stop all", "unsubscribe", "Opt out"],
            Intent::Stop,
        );
        all(&["help", "INFO", "?"], Intent::Help);
    }

    #[test]
    fn test_welsh_keywords_with_and_without_accents() {
        all(
            &["ie", "Iawn", "ydw", "cadarnhau", "Cadarnhâu"],
            Intent::Confirm,
        );
        all(
            &["na", "Nac", "dim", "canslo", "Ni allaf ddod"],
            Intent::Cancel,
        );
        all(
            &["newid", "Aildrefnu", "symud", "amser arall"],
            Intent::Reschedule,
        );
        all(&["stopio", "diwedd"], Intent::Stop);
        all(&["cymorth"], Intent::Help);
    }

    #[test]
    fn test_a_polite_word_after_the_keyword_is_allowed() {
        assert_eq!(parse("yes thanks"), Intent::Confirm);
        assert_eq!(parse("Cancel please"), Intent::Cancel);
        assert_eq!(parse("iawn diolch"), Intent::Confirm);
        assert_eq!(parse("canslo os gwelwch yn dda"), Intent::Cancel);
        assert_eq!(
            parse("thanks"),
            Intent::Unknown,
            "a polite word alone means nothing"
        );
    }

    #[test]
    fn test_anything_that_is_not_just_a_keyword_is_unknown_so_we_ask_again() {
        for w in [
            "no problem",
            "yes but can I come later",
            "I will be late",
            "cancel my other appointment",
            "",
            "   ",
            "👍",
            "ok see you tomorrow",
            "not sure",
            "y n",
        ] {
            assert_eq!(parse(w), Intent::Unknown, "{w:?}");
        }
    }

    #[test]
    fn test_a_single_digit_is_a_choice_and_nothing_else_is() {
        assert_eq!(parse("1"), Intent::Choice(1));
        assert_eq!(parse(" 2. "), Intent::Choice(2));
        assert_eq!(parse("0"), Intent::Unknown);
        assert_eq!(parse("10"), Intent::Unknown);
        assert_eq!(parse("1 2"), Intent::Unknown);
    }

    #[test]
    fn test_stop_always_beats_politeness_and_is_never_missed_in_either_language() {
        // An opt-out must not be lost to a parsing quirk.
        for w in [
            "STOP",
            "stop.",
            "Stop!",
            "stop please",
            "STOPIO",
            "stopio diolch",
        ] {
            assert_eq!(parse(w), Intent::Stop, "{w:?}");
        }
    }

    #[test]
    fn test_fold_removes_accents_apostrophes_and_extra_space() {
        assert_eq!(fold("  Can’t   COME!! "), "cant come");
        assert_eq!(fold("Cadarnhâu"), "cadarnhau");
        assert_eq!(fold("Ŵ ŷ"), "w y");
    }
}

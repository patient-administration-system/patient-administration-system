//! How many SMS segments a text costs (PILOT-5).
//!
//! A text made only of GSM 7-bit characters fits 160 characters in one segment (153 per segment when
//! split). One character outside that set switches the **whole message** to UCS-2: 70 characters in one
//! segment (67 per segment when split). Welsh needs this more than English does: `â ê î ô û ŵ ŷ` (the
//! circumflex vowels) and the other accented capitals are not in the GSM alphabet, so a Welsh reminder
//! with a single `ŵ` costs more than twice the segments of the same length in English. Knowing that
//! lets an administrator trim a template before it is sent thousands of times.
//!
//! Characters in the GSM extension table (`€ [ ] { } \ ^ | ~` and form feed) count as two.
//! Characters outside the Basic Multilingual Plane (emoji) are two UCS-2 units.

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Gsm7,
    Ucs2,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct SmsLength {
    pub encoding: Encoding,
    /// Length in the encoding's own units (septets, or UTF-16 code units).
    pub units: usize,
    pub segments: usize,
    /// The first character that forced UCS-2, when it did.
    #[schema(required = true)]
    pub forced_by: Option<char>,
}

/// GSM 03.38 basic character set (one septet each).
const GSM_BASIC: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
/// GSM 03.38 extension table (two septets each: escape plus the character).
const GSM_EXT: &str = "\u{0C}^{}\\[~]|€";

fn gsm_units(c: char) -> Option<usize> {
    if GSM_BASIC.contains(c) {
        Some(1)
    } else if GSM_EXT.contains(c) {
        Some(2)
    } else {
        None
    }
}

pub fn analyze(text: &str) -> SmsLength {
    let mut septets = 0usize;
    let mut forced_by = None;
    for c in text.chars() {
        match gsm_units(c) {
            Some(n) => septets += n,
            None => {
                forced_by = Some(c);
                break;
            }
        }
    }
    if forced_by.is_none() {
        return SmsLength {
            encoding: Encoding::Gsm7,
            units: septets,
            segments: segments(septets, 160, 153),
            forced_by,
        };
    }
    let units = text.encode_utf16().count();
    SmsLength {
        encoding: Encoding::Ucs2,
        units,
        segments: segments(units, 70, 67),
        forced_by,
    }
}

fn segments(units: usize, single: usize, multi: usize) -> usize {
    match units {
        0 => 0,
        n if n <= single => 1,
        n => n.div_ceil(multi),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_english_is_gsm7_with_160_in_one_segment() {
        let a = analyze("Hello Ada, your appointment is on Tue 3 Nov 09:30.");
        assert_eq!(
            (a.encoding, a.segments, a.forced_by),
            (Encoding::Gsm7, 1, None)
        );
        assert_eq!(analyze(&"a".repeat(160)).segments, 1);
        assert_eq!(
            analyze(&"a".repeat(161)).segments,
            2,
            "split messages carry 153 each"
        );
        assert_eq!(analyze(&"a".repeat(306)).segments, 2);
        assert_eq!(analyze(&"a".repeat(307)).segments, 3);
        assert_eq!(analyze("").segments, 0);
    }

    #[test]
    fn test_one_welsh_circumflex_makes_the_whole_message_ucs2() {
        let en = "a".repeat(100);
        assert_eq!(analyze(&en).segments, 1);
        // The same 100 characters with one ŵ: 70 per segment, 67 once split.
        let cy = format!("{}ŵ", "a".repeat(99));
        let a = analyze(&cy);
        assert_eq!(
            (a.encoding, a.units, a.segments, a.forced_by),
            (Encoding::Ucs2, 100, 2, Some('ŵ'))
        );
        assert_eq!(
            analyze(&format!("{}ŵ", "a".repeat(69))).segments,
            1,
            "70 units fits one"
        );
        assert_eq!(analyze(&format!("{}ŵ", "a".repeat(70))).segments, 2);
        assert_eq!(
            analyze(&format!("{}â", "a".repeat(133))).segments,
            2,
            "134 units = 2 x 67"
        );
        assert_eq!(analyze(&format!("{}â", "a".repeat(134))).segments, 3);
    }

    #[test]
    fn test_welsh_letters_that_are_in_the_gsm_alphabet_do_not_force_ucs2() {
        // é è ù ì ò ä ö ü are GSM; â ê î ô û ŵ ŷ are not.
        assert_eq!(analyze("café ù ò ä ö ü").encoding, Encoding::Gsm7);
        for c in ['â', 'ê', 'î', 'ô', 'û', 'ŵ', 'ŷ', 'Â', 'Ŵ'] {
            assert_eq!(analyze(&format!("a{c}")).encoding, Encoding::Ucs2, "{c}");
        }
    }

    #[test]
    fn test_extension_characters_cost_two_and_emoji_cost_two_units() {
        assert_eq!(analyze("€").units, 2);
        assert_eq!(analyze("[x]").units, 5);
        assert_eq!(analyze(&"€".repeat(80)).segments, 1, "160 septets");
        assert_eq!(analyze(&"€".repeat(81)).segments, 2);
        let e = analyze("hi 👍");
        assert_eq!(
            (e.encoding, e.units),
            (Encoding::Ucs2, 5),
            "👍 is two UTF-16 units"
        );
    }
}

//! The NHS number (England, Wales and the Isle of Man): one identifier scheme among any others (QUAL-40).

use super::Scheme;
use crate::{Error, Result};

/// The scheme, as [`super::SCHEMES`] lists it.
pub const SCHEME: Scheme = Scheme {
    code: "gb-nhs-number",
    system: SYSTEM,
    name: "NHS number",
    check,
    format,
};

/// The FHIR identifier system for an NHS number.
pub const SYSTEM: &str = "https://fhir.nhs.uk/Id/nhs-number";

/// Check an NHS number (DTAC-8): ten digits whose last is the modulus-11 check digit of the first nine
/// (weights 10 down to 2; `11 - (sum mod 11)`, where 11 becomes 0 and 10 makes the number invalid). Spaces and
/// hyphens are allowed in the input. Returns the ten digits. It checks the number is well formed; it does not
/// say the number is issued or belongs to this patient (that needs a PDS trace, which the PAS does not do:
/// see `governance/pds-decision.md`).
pub fn check(raw: &str) -> Result<String> {
    let digits: String = raw.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation("an NHS number is ten digits"));
    }
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    // Weights 10 down to 2 over the first nine digits; the digits are at most 9, so nothing here can overflow,
    // and if it somehow did the number is not valid.
    let remainder = d
        .iter()
        .take(9)
        .zip((2..=10_u32).rev())
        .try_fold(0_u32, |acc, (x, w)| {
            x.checked_mul(w).and_then(|p| acc.checked_add(p))
        })
        .and_then(|sum| sum.checked_rem(11));
    let check = match remainder.and_then(|r| 11_u32.checked_sub(r)) {
        Some(11) => 0,
        Some(c) if c < 10 => c,
        _ => return Err(Error::validation("that is not a valid NHS number")),
    };
    if d.get(9) != Some(&check) {
        return Err(Error::validation(
            "that is not a valid NHS number (the check digit does not match)",
        ));
    }
    Ok(digits)
}

/// The NHS number written the usual way, `943 476 5919`. The input must already be valid.
pub fn format(valid: &str) -> String {
    let part = |from: usize, to: usize| valid.get(from..to).unwrap_or_default();
    format!("{} {} {}", part(0, 3), part(3, 6), part(6, valid.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_valid_numbers_pass_with_or_without_spaces() {
        // 943 476 5919 is the example in the NHS number specification.
        assert_eq!(check("9434765919").unwrap(), "9434765919");
        assert_eq!(check("943 476 5919").unwrap(), "9434765919");
        assert_eq!(check("943-476-5919").unwrap(), "9434765919");
        assert_eq!(format("9434765919"), "943 476 5919");
    }

    #[test]
    fn test_a_wrong_check_digit_fails() {
        for n in ["9434765918", "9434765910", "1234567890"] {
            assert!(check(n).is_err(), "{n}");
        }
    }

    #[test]
    fn test_a_check_of_eleven_is_zero_and_a_check_of_ten_is_invalid() {
        // 0000000000: sum 0, 11 - 0 = 11 -> check 0.
        assert!(check("0000000000").is_ok());
        // 9990000000-ish: find a number whose check would be 10 and see it refused whatever the last digit.
        let mut found = false;
        for n in 0..2000u32 {
            let first9 = format!("{n:09}");
            let d: Vec<u32> = first9.chars().map(|c| c.to_digit(10).unwrap()).collect();
            let sum: u32 = d.iter().zip((2..=10).rev()).map(|(x, w)| x * w).sum();
            if 11 - (sum % 11) == 10 {
                found = true;
                for last in 0..=9 {
                    assert!(check(&format!("{first9}{last}")).is_err());
                }
                break;
            }
        }
        assert!(found);
    }

    #[test]
    fn test_wrong_length_and_non_digits_fail() {
        for n in [
            "",
            "943476591",
            "94347659190",
            "94347659l9",
            "abcdefghij",
            "943 476 591",
        ] {
            assert!(check(n).is_err(), "{n}");
        }
    }
}

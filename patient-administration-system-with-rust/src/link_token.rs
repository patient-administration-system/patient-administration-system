//! Single-use link tokens (QUAL-29).
//!
//! Reply links (CAP-13), backfill offers (PILOT-8) and the voice keypad (which reuses reply links) all
//! hand a patient an opaque token and store only its hash. The token carries no personal data; the server
//! looks it up by hash, so a leaked database does not leak working links.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A fresh token: two random UUIDs, 64 lowercase hex characters, 244 random bits.
pub fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// The stored form of a token: SHA-256, lowercase hex.
pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// How an action taken through a link is named in the audit log, e.g. `link:` + this: the first
/// 12 characters of the hash, enough to find the row and useless for rebuilding the link.
pub fn audit_tag(token: &str) -> String {
    let mut h = hash(token);
    h.truncate(12);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_64_lowercase_hex_characters_and_never_repeats() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_ne!(a, b);
    }

    #[test]
    fn the_hash_is_sha256_hex() {
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash(&new_token()).len(), 64);
    }

    #[test]
    fn the_audit_tag_is_the_start_of_the_hash() {
        let t = new_token();
        assert_eq!(audit_tag(&t), hash(&t)[..12]);
    }
}

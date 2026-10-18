//! Authentication of inbound replies from message carriers (PILOT-3).
//!
//! A carrier (an SMS or WhatsApp provider, or a batch uploader) is not a person and holds no PASETO
//! token. It proves a reply is genuine by signing it with a secret that belongs to that carrier alone:
//!
//! ```text
//! X-Carrier-Id:        <carrier id>
//! X-Carrier-Timestamp: <unix seconds>
//! X-Carrier-Signature: sha256=<hex HMAC-SHA256(secret, "<timestamp>." + raw body)>
//! ```
//!
//! - **One credential per carrier**, so a leaked secret is revoked for one carrier and the audit log
//!   says which carrier sent what (`carrier:<id>`).
//! - **Replay window.** A timestamp more than [`REPLAY_WINDOW_SECS`] from now (either way) is refused,
//!   so a captured request cannot be replayed later. Inside the window a replay does nothing anyway:
//!   replies are idempotent by `message_id` (CAP-2).
//! - **Rotation.** A carrier may have several secrets at once (`a|b`); any one verifies. Add the new
//!   one, move the carrier over, then remove the old one.
//! - Comparison is constant-time. The failure reason is logged, never returned: the caller learns
//!   only "not authenticated".
//!
//! Configured with `PAS_CARRIER_SECRETS="twilio=s3cr3t,whatsapp=old|new"`.

use std::collections::HashMap;

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const ID_HEADER: &str = "x-carrier-id";
pub const TIMESTAMP_HEADER: &str = "x-carrier-timestamp";
pub const SIGNATURE_HEADER: &str = "x-carrier-signature";
/// Five minutes either way: generous for clock skew, short for replay.
pub const REPLAY_WINDOW_SECS: i64 = 300;
/// Shorter secrets are refused at start-up: an HMAC key is only as good as its entropy.
pub const MIN_SECRET_LEN: usize = 16;

#[derive(Debug, PartialEq, Eq)]
pub enum CarrierError {
    UnknownCarrier,
    BadTimestamp,
    OutsideWindow,
    BadSignature,
}

/// carrier id -> accepted secrets.
#[derive(Debug, Clone, Default)]
pub struct Carriers(HashMap<String, Vec<Vec<u8>>>);

impl Carriers {
    /// Parse `id=secret[|secret2],id2=secret`. Refuses short secrets and duplicate ids.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut map = HashMap::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (id, secrets) = item
                .split_once('=')
                .ok_or_else(|| format!("PAS_CARRIER_SECRETS entry {item:?} is not id=secret"))?;
            let id = id.trim();
            if id.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err(format!(
                    "carrier id {id:?} must be letters, digits, '-', '_' or '.'"
                ));
            }
            let list: Vec<Vec<u8>> = secrets.split('|').map(|s| s.as_bytes().to_vec()).collect();
            if list.iter().any(|s| s.len() < MIN_SECRET_LEN) {
                return Err(format!(
                    "every secret for carrier {id} must be at least {MIN_SECRET_LEN} characters"
                ));
            }
            if map.insert(id.to_string(), list).is_some() {
                return Err(format!("carrier {id} is listed twice"));
            }
        }
        Ok(Self(map))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Verify a request. `now` is unix seconds.
    pub fn verify(
        &self,
        id: &str,
        timestamp: &str,
        signature: &str,
        body: &[u8],
        now: i64,
    ) -> Result<(), CarrierError> {
        let secrets = self.0.get(id).ok_or(CarrierError::UnknownCarrier)?;
        let ts: i64 = timestamp
            .trim()
            .parse()
            .map_err(|_| CarrierError::BadTimestamp)?;
        if (now - ts).abs() > REPLAY_WINDOW_SECS {
            return Err(CarrierError::OutsideWindow);
        }
        let want = signature
            .trim()
            .strip_prefix("sha256=")
            .and_then(unhex)
            .ok_or(CarrierError::BadSignature)?;
        for secret in secrets {
            let mut mac =
                <Hmac<Sha256>>::new_from_slice(secret).map_err(|_| CarrierError::BadSignature)?;
            mac.update(timestamp.trim().as_bytes());
            mac.update(b".");
            mac.update(body);
            if mac.verify_slice(&want).is_ok() {
                return Ok(());
            }
        }
        Err(CarrierError::BadSignature)
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Sign a body the way a carrier must (used by tests and by anyone writing a carrier adapter).
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret).expect("any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    format!(
        "sha256={}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "0123456789abcdef-one";
    const S2: &str = "0123456789abcdef-two";
    fn carriers() -> Carriers {
        Carriers::parse(&format!("sms={S1},wa={S1}|{S2}")).unwrap()
    }

    #[test]
    fn test_a_correctly_signed_request_inside_the_window_verifies() {
        let c = carriers();
        let sig = sign(S1.as_bytes(), 1000, b"{}");
        assert_eq!(c.verify("sms", "1000", &sig, b"{}", 1000), Ok(()));
        assert_eq!(
            c.verify("sms", "1000", &sig, b"{}", 1000 + REPLAY_WINDOW_SECS),
            Ok(())
        );
        assert_eq!(
            c.verify("sms", "1000", &sig, b"{}", 1000 - REPLAY_WINDOW_SECS),
            Ok(())
        );
    }

    #[test]
    fn test_old_future_and_unparseable_timestamps_are_refused() {
        let c = carriers();
        let sig = sign(S1.as_bytes(), 1000, b"{}");
        assert_eq!(
            c.verify("sms", "1000", &sig, b"{}", 1000 + REPLAY_WINDOW_SECS + 1),
            Err(CarrierError::OutsideWindow)
        );
        assert_eq!(
            c.verify("sms", "1000", &sig, b"{}", 1000 - REPLAY_WINDOW_SECS - 1),
            Err(CarrierError::OutsideWindow)
        );
        assert_eq!(
            c.verify("sms", "soon", &sig, b"{}", 1000),
            Err(CarrierError::BadTimestamp)
        );
    }

    #[test]
    fn test_a_changed_body_timestamp_or_carrier_secret_fails() {
        let c = carriers();
        let sig = sign(S1.as_bytes(), 1000, b"{\"a\":1}");
        assert_eq!(
            c.verify("sms", "1000", &sig, b"{\"a\":2}", 1000),
            Err(CarrierError::BadSignature)
        );
        // The timestamp is signed too, so it cannot be bumped to dodge the window.
        assert_eq!(
            c.verify("sms", "1001", &sig, b"{\"a\":1}", 1001),
            Err(CarrierError::BadSignature)
        );
        // One carrier's secret does not work as another's (the second secret belongs to `wa` only).
        let wa_only = sign(S2.as_bytes(), 1000, b"x");
        assert_eq!(
            c.verify("sms", "1000", &wa_only, b"x", 1000),
            Err(CarrierError::BadSignature)
        );
        assert_eq!(c.verify("wa", "1000", &wa_only, b"x", 1000), Ok(()));
        assert_eq!(
            c.verify("nobody", "1000", &sig, b"x", 1000),
            Err(CarrierError::UnknownCarrier)
        );
    }

    #[test]
    fn test_malformed_signatures_are_refused_without_panicking() {
        let c = carriers();
        for bad in [
            "",
            "sha256=",
            "sha256=zz",
            "sha256=abc",
            "md5=00",
            "00".repeat(32).as_str(),
            "sha256=é",
        ] {
            assert_eq!(
                c.verify("sms", "1000", bad, b"x", 1000),
                Err(CarrierError::BadSignature),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn test_configuration_refuses_weak_or_ambiguous_secrets() {
        assert!(Carriers::parse("sms=short").is_err());
        assert!(Carriers::parse(&format!("sms={S1}|short")).is_err());
        assert!(Carriers::parse(&format!("sms={S1},sms={S2}")).is_err());
        assert!(Carriers::parse(&format!("bad id={S1}")).is_err());
        assert!(Carriers::parse("justwords").is_err());
        assert!(Carriers::parse("").unwrap().is_empty());
    }
}

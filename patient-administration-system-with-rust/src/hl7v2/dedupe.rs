//! A resent HL7 v2 message is not processed twice (NFR-31, HUB-2).
//!
//! A sender that does not get our ACK (a dropped connection, a timeout) sends the same message again, with
//! the same MSH-10. Without this, an A01 would admit the patient twice. Each message that changes something
//! is keyed on its sending application (MSH-3), sending facility (MSH-4) and a SHA-256 of its control id
//! (MSH-10), and goes through [`claim`] before it is processed and [`finish`] after:
//!
//! - **New:** the key is claimed (`processing`) and the message is processed.
//! - **Accepted before:** an `AA` was stored for the key; the resend gets that ACK again (its code and text,
//!   rebuilt for the resend's own header) and nothing is processed.
//! - **Not finished:** a claim is still `processing`, because a copy is being processed now, or one stopped
//!   part-way (a crash, a lost database connection) after its change may have been committed. It is refused
//!   with an AE and logged, and a person must check: processing it could admit twice, which is worse than a
//!   message that waits (fail closed).
//! - **AE or AR:** the claim is removed, because nothing was changed, so a corrected resend is processed.
//!
//! Only a hash of MSH-10 is kept: some senders build it from a patient identifier. A message with no MSH-10
//! cannot be recognised and is processed as it comes. Keys are kept `PAS_HL7_DEDUPE_DAYS` (default 7) and then
//! removed by [`purge`]; a resend after that is processed again.

use sha2::{Digest, Sha256};

use super::ack::{AckCode, Inbound};
use crate::Result;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Param, Pool, Stmt};

/// What identifies one message from one sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    sending_app: String,
    sending_facility: String,
    control_id_sha256: String,
}

impl Key {
    /// The key of `inbound`, or `None` when it has no control id to recognise it by.
    pub fn of(inbound: &Inbound) -> Option<Self> {
        if inbound.control_id.trim().is_empty() {
            return None;
        }
        let digest = Sha256::digest(inbound.control_id.as_bytes());
        Some(Self {
            sending_app: inbound.sending_app.clone(),
            sending_facility: inbound.sending_facility.clone(),
            control_id_sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        })
    }

    /// A short form for logs: the first 12 hex digits of the hash, never the control id.
    pub fn short(&self) -> &str {
        self.control_id_sha256
            .get(..12)
            .unwrap_or(&self.control_id_sha256)
    }
}

/// What [`claim`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Claimed: process the message, then call [`finish`].
    New,
    /// Accepted before: answer with this code and MSA-3 text, and process nothing.
    Replay(AckCode, Option<String>),
    /// A copy has not finished: refuse it.
    Unfinished,
}

fn days() -> i64 {
    crate::config::env::int("PAS_HL7_DEDUPE_DAYS")
}

/// Claim `key` for a message about to be processed.
pub async fn claim(db: &Pool, key: &Key) -> Result<Claim> {
    let binds = || {
        [
            key.sending_app.clone().into(),
            key.sending_facility.clone().into(),
            key.control_id_sha256.clone().into(),
        ]
    };
    // A key older than the retention period is forgotten first, as `purge` would have done.
    db.run(Stmt::new(
        "DELETE FROM hl7_inbound_messages WHERE sending_app = $1 AND sending_facility = $2 \
         AND control_id_sha256 = $3 AND received_at < now() - make_interval(days => $4::int)",
        [
            key.sending_app.clone().into(),
            key.sending_facility.clone().into(),
            key.control_id_sha256.clone().into(),
            days().into(),
        ],
    ))
    .await?;
    let claimed = db
        .one(Stmt::new(
            "INSERT INTO hl7_inbound_messages (sending_app, sending_facility, control_id_sha256, state) \
             VALUES ($1, $2, $3, 'processing') ON CONFLICT DO NOTHING RETURNING state",
            binds(),
        ))
        .await?;
    if claimed.is_some() {
        return Ok(Claim::New);
    }
    let Some(row) = db
        .one(Stmt::new(
            "SELECT state, ack_code, ack_text FROM hl7_inbound_messages \
             WHERE sending_app = $1 AND sending_facility = $2 AND control_id_sha256 = $3",
            binds(),
        ))
        .await?
    else {
        // Removed between the two statements (a finish after an AE): the caller may simply send again.
        return Ok(Claim::Unfinished);
    };
    let state: String = row.col("state")?;
    if state != "done" {
        return Ok(Claim::Unfinished);
    }
    let code: Option<String> = row.col("ack_code")?;
    let text: Option<String> = row.col("ack_text")?;
    Ok(match code.as_deref().and_then(AckCode::parse) {
        Some(code) => Claim::Replay(code, text),
        None => Claim::Unfinished,
    })
}

/// Record how the claimed message was answered: an `AA` is kept for replay, anything else frees the key.
pub async fn finish(db: &Pool, key: &Key, code: AckCode, text: Option<&str>) -> Result<()> {
    let binds: [Param; 3] = [
        key.sending_app.clone().into(),
        key.sending_facility.clone().into(),
        key.control_id_sha256.clone().into(),
    ];
    let stmt = if code == AckCode::Accept {
        Stmt::new(
            "UPDATE hl7_inbound_messages SET state = 'done', ack_code = $4, ack_text = $5, finished_at = now() \
             WHERE sending_app = $1 AND sending_facility = $2 AND control_id_sha256 = $3",
            [
                binds[0].clone(),
                binds[1].clone(),
                binds[2].clone(),
                code.as_str().into(),
                text.map(str::to_string).into(),
            ],
        )
    } else {
        Stmt::new(
            "DELETE FROM hl7_inbound_messages \
             WHERE sending_app = $1 AND sending_facility = $2 AND control_id_sha256 = $3",
            binds,
        )
    };
    db.run(stmt).await?;
    Ok(())
}

/// Remove keys older than `PAS_HL7_DEDUPE_DAYS`. Returns how many.
pub async fn purge(db: &Pool) -> Result<u64> {
    let done = db
        .run(Stmt::new(
            "DELETE FROM hl7_inbound_messages WHERE received_at < now() - make_interval(days => $1::int)",
            [days().into()],
        ))
        .await?;
    Ok(done.rows_affected())
}

/// Process `inbound` at most once: claim it, run `process` (which returns the ACK it built) when the claim is
/// new, and record the answer. `process` is not run for a replay or an unfinished copy.
pub async fn guarded<F>(db: &Pool, inbound: &Inbound, process: F) -> String
where
    F: std::future::Future<Output = String>,
{
    crate::metrics::hl7_received();
    let Some(key) = Key::of(inbound) else {
        return process.await;
    };
    match claim(db, &key).await {
        Ok(Claim::New) => {}
        Ok(Claim::Replay(code, text)) => {
            tracing::info!(target: "pas::hl7v2::dedupe", key = key.short(), "resent message answered from the first ACK; not processed again");
            return super::ack::ack_to(inbound, code, text.as_deref());
        }
        Ok(Claim::Unfinished) => {
            tracing::warn!(target: "pas::hl7v2::dedupe", key = key.short(), "a copy of this message has not finished; refused, a person must check");
            return super::ack::ack_to(
                inbound,
                AckCode::AppError,
                Some("an earlier copy has not finished; not processed again"),
            );
        }
        Err(e) => {
            tracing::error!(target: "pas::hl7v2::dedupe", "claim failed: {e}");
            return super::ack::ack_to(
                inbound,
                AckCode::AppError,
                Some("internal error; send again"),
            );
        }
    }
    let ack = process.await;
    let (code, text) = super::ack::ack_code_and_text(&ack).unwrap_or((AckCode::AppError, None));
    if let Err(e) = finish(db, &key, code, text.as_deref()).await {
        // The message was processed; its key stays `processing`, so a resend is refused, not processed again.
        tracing::error!(target: "pas::hl7v2::dedupe", key = key.short(), "could not record the answer: {e}");
    }
    ack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_holds_a_hash_of_the_control_id_never_the_id() {
        let inbound = Inbound {
            sending_app: "EMR".into(),
            sending_facility: "WARDS".into(),
            control_id: "ID9434765919-001".into(),
            ..Inbound::default()
        };
        let key = Key::of(&inbound).unwrap();
        assert_eq!(key.control_id_sha256.len(), 64);
        assert!(!format!("{key:?}").contains("9434765919"));
        assert_eq!(key.short().len(), 12);
    }

    #[test]
    fn no_control_id_means_no_key() {
        assert_eq!(Key::of(&Inbound::default()), None);
        let blank = Inbound {
            control_id: "  ".into(),
            ..Inbound::default()
        };
        assert_eq!(Key::of(&blank), None);
    }

    #[test]
    fn the_key_differs_by_sender_and_by_id() {
        let a = Inbound {
            sending_app: "EMR".into(),
            control_id: "1".into(),
            ..Inbound::default()
        };
        let b = Inbound {
            sending_app: "LAB".into(),
            ..a.clone()
        };
        let c = Inbound {
            control_id: "2".into(),
            ..a.clone()
        };
        assert_ne!(Key::of(&a), Key::of(&b));
        assert_ne!(Key::of(&a), Key::of(&c));
    }
}

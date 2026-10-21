//! HL7 v2 MLLP outbound publisher.
//!
//! Subscribes to the transactional outbox via [`EventPublisher`]; when the
//! dispatcher hands an event in, this publisher:
//!
//! 1. Loads the underlying domain row (Encounter / Appointment / Bed) from
//!    the local PAS database.
//! 2. Hydrates the patient (and optionally the practitioner) from
//!    `person-service` / `worker-service` via [`PersonClient`] / [`WorkerClient`].
//! 3. Builds the appropriate HL7 v2 wire frame using the encoders in
//!    [`crate::hl7v2::encode`].
//! 4. Opens a fresh MLLP TCP connection to `peer`, sends the frame, awaits
//!    the ACK with a bounded timeout, parses MSA-1, and returns:
//!    - `AA` → `Ok(())`.
//!    - `AE` / `AR` / no ACK / connect failure → `Err(Error::Streaming)`,
//!      which causes the outbox dispatcher to bump `retry_count` and
//!      eventually DLQ the row.
//!
//! **Scope.** The publisher handles every event the PAS API emits today
//! that has a standard ADT or SIU trigger. Event types that don't have a
//! 1:1 wire mapping (e.g. `LetterGenerated`, `BedStatusChanged`,
//! `RTTClockStarted`, billing / coverage / waitlist events) are logged
//! at debug and acknowledged with `Ok(())` — the outbox row + webhook
//! sink remain the durable record. Adding a new mapping is a one-line
//! addition to `dispatch_to_trigger` plus an encoder.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::hl7v2::encode::{EncodeContext, encode_adt, encode_siu};
use crate::hl7v2::mllp;
use crate::hl7v2::{SegmentRead, parse_message};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::upstream::{PersonClient, UserContext, WorkerClient};
use crate::{Error, Result};

const ACK_TIMEOUT_SECS: u64 = 5;
const CONNECT_TIMEOUT_SECS: u64 = 3;

/// Outbound MLLP publisher. Build with [`Hl7v2MllpPublisher::new`] and
/// fluent-attach upstream clients via [`Hl7v2MllpPublisher::with_person_client`]
/// / [`Hl7v2MllpPublisher::with_worker_client`].
pub struct Hl7v2MllpPublisher {
    db: DatabaseConnection,
    peer: String,
    sender: String,
    receiver: String,
    person_client: Option<Arc<dyn PersonClient>>,
    worker_client: Option<Arc<dyn WorkerClient>>,
}

impl Hl7v2MllpPublisher {
    /// Build a publisher with no upstream clients. Without at least a
    /// `PersonClient` the publisher will fail every encounter/appointment
    /// event with `Error::Streaming("person_client unset")` — main wires
    /// it from the same `PERSON_SERVICE_URL` it uses for the REST proxies.
    pub fn new(db: DatabaseConnection, peer: String, sender: &str, receiver: &str) -> Self {
        Self {
            db,
            peer,
            sender: sender.to_string(),
            receiver: receiver.to_string(),
            person_client: None,
            worker_client: None,
        }
    }

    pub fn with_person_client(mut self, c: Arc<dyn PersonClient>) -> Self {
        self.person_client = Some(c);
        self
    }

    pub fn with_worker_client(mut self, c: Arc<dyn WorkerClient>) -> Self {
        self.worker_client = Some(c);
        self
    }

    async fn send_frame(&self, payload: &[u8]) -> Result<String> {
        let connect = TcpStream::connect(&self.peer);
        let stream = timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS), connect)
            .await
            .map_err(|_| Error::Streaming(format!("MLLP connect to {} timed out", self.peer)))?
            .map_err(|e| Error::Streaming(format!("MLLP connect to {}: {e}", self.peer)))?;
        let (mut r, mut w) = stream.into_split();
        mllp::write_frame(&mut w, payload).await?;
        let ack = timeout(
            Duration::from_secs(ACK_TIMEOUT_SECS),
            mllp::read_frame(&mut r),
        )
        .await
        .map_err(|_| Error::Streaming("MLLP ACK timeout".to_string()))??
        .ok_or_else(|| Error::Streaming("MLLP peer closed before ACK".to_string()))?;
        let ack_str = String::from_utf8(ack)
            .map_err(|e| Error::Streaming(format!("MLLP ACK is not UTF-8: {e}")))?;
        Ok(ack_str)
    }

    fn check_ack(ack: &str) -> Result<()> {
        let msg =
            parse_message(ack).map_err(|e| Error::Streaming(format!("ACK parse failed: {e}")))?;
        let msa = msg
            .segment("MSA")
            .ok_or_else(|| Error::Streaming("ACK missing MSA segment".to_string()))?;
        let code = msa.raw(1);
        match code.as_str() {
            "AA" => Ok(()),
            "AE" | "AR" => Err(Error::Streaming(format!(
                "peer rejected with {code}: {}",
                msa.raw(3)
            ))),
            other => Err(Error::Streaming(format!("unknown ACK code {other:?}"))),
        }
    }

    async fn dispatch(&self, event: &DomainEvent) -> Result<Option<String>> {
        let ctx = UserContext {
            user_id: Some("pas-publisher".into()),
            user_ip: None,
            user_agent: Some("pas-mllp-publisher/0.44".into()),
        };
        let control_id = event.id.to_string();
        let now = chrono::Utc::now();

        match event.event_type.as_str() {
            "EncounterRegistered" => self.adt(event, "A04", &control_id, now, &ctx).await,
            "EncounterAdmitted" => self.adt(event, "A01", &control_id, now, &ctx).await,
            "EncounterPreAdmitted" => self.adt(event, "A05", &control_id, now, &ctx).await,
            "EncounterPreAdmitCancelled" => self.adt(event, "A38", &control_id, now, &ctx).await,
            "EncounterPromotedToInpatient" => self.adt(event, "A06", &control_id, now, &ctx).await,
            "EncounterDemotedToOutpatient" => self.adt(event, "A07", &control_id, now, &ctx).await,
            "EncounterCancelled" => self.adt(event, "A11", &control_id, now, &ctx).await,
            "EncounterTransferred" => self.adt(event, "A02", &control_id, now, &ctx).await,
            "EncounterTransferCancelled" => self.adt(event, "A12", &control_id, now, &ctx).await,
            "EncounterDischarged" => self.adt(event, "A03", &control_id, now, &ctx).await,
            "EncounterDischargeCancelled" => self.adt(event, "A13", &control_id, now, &ctx).await,
            "AppointmentBooked" => self.siu(event, "S12", &control_id, now, &ctx).await,
            "AppointmentRescheduled" => self.siu(event, "S13", &control_id, now, &ctx).await,
            "AppointmentModified" => self.siu(event, "S14", &control_id, now, &ctx).await,
            "AppointmentCancelled" => self.siu(event, "S15", &control_id, now, &ctx).await,
            "AppointmentNoShow" => self.siu(event, "S26", &control_id, now, &ctx).await,
            other => {
                tracing::debug!(
                    event_type = %other,
                    event_id = %event.id,
                    "no HL7 v2 wire mapping; outbox row + webhook sink remain authoritative"
                );
                Ok(None)
            }
        }
    }

    async fn adt(
        &self,
        event: &DomainEvent,
        trigger: &str,
        control_id: &str,
        now: chrono::DateTime<chrono::Utc>,
        ctx: &UserContext,
    ) -> Result<Option<String>> {
        let pc = self
            .person_client
            .as_deref()
            .ok_or_else(|| Error::Streaming("person_client unset".into()))?;
        let encounter_id = extract_uuid(&event.payload, "encounter_id").ok_or_else(|| {
            Error::Streaming(format!("{} payload missing encounter_id", event.event_type))
        })?;
        let enc = EncounterRepository::find_by_id(&self.db, encounter_id)
            .await?
            .ok_or_else(|| Error::Streaming(format!("encounter {encounter_id} not found")))?;
        let bed = match extract_uuid(&event.payload, "bed_id") {
            Some(bid) => BedRepository::find_by_id(&self.db, bid).await?,
            None => None,
        };
        let person = pc.get(enc.person_id, ctx).await?;
        let enc_ctx = EncodeContext {
            sender: &self.sender,
            receiver: &self.receiver,
            trigger,
            control_id,
            now,
        };
        Ok(Some(encode_adt(enc_ctx, &person, &enc, bed.as_ref())))
    }

    async fn siu(
        &self,
        event: &DomainEvent,
        trigger: &str,
        control_id: &str,
        now: chrono::DateTime<chrono::Utc>,
        ctx: &UserContext,
    ) -> Result<Option<String>> {
        let pc = self
            .person_client
            .as_deref()
            .ok_or_else(|| Error::Streaming("person_client unset".into()))?;
        let appointment_id = extract_uuid(&event.payload, "appointment_id").ok_or_else(|| {
            Error::Streaming(format!(
                "{} payload missing appointment_id",
                event.event_type
            ))
        })?;
        let appt = AppointmentRepository::find_by_id(&self.db, appointment_id)
            .await?
            .ok_or_else(|| Error::Streaming(format!("appointment {appointment_id} not found")))?;
        let person = pc.get(appt.person_id, ctx).await?;
        let enc_ctx = EncodeContext {
            sender: &self.sender,
            receiver: &self.receiver,
            trigger,
            control_id,
            now,
        };
        Ok(Some(encode_siu(enc_ctx, &person, &appt, None)))
    }
}

#[async_trait]
impl EventPublisher for Hl7v2MllpPublisher {
    async fn publish(&self, event: DomainEvent) -> Result<()> {
        let frame = match self.dispatch(&event).await? {
            Some(f) => f,
            None => return Ok(()),
        };
        let ack = self.send_frame(frame.as_bytes()).await?;
        Self::check_ack(&ack)
    }
}

fn extract_uuid(v: &serde_json::Value, key: &str) -> Option<Uuid> {
    v.get(key)
        .and_then(|x| x.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_check_ack_accepts_aa() {
        let ack = "MSH|^~\\&|PAS|FAC|EMR|FAC|20260531120000||ACK|MSG1|P|2.5\rMSA|AA|MSG1\r";
        Hl7v2MllpPublisher::cls_check_ack(ack).expect("AA must pass");
    }

    #[test]
    fn test_check_ack_rejects_ae_with_diagnostics() {
        let ack =
            "MSH|^~\\&|PAS|FAC|EMR|FAC|20260531120000||ACK|MSG2|P|2.5\rMSA|AE|MSG2|PID-5 missing\r";
        let err = Hl7v2MllpPublisher::cls_check_ack(ack).expect_err("AE must fail");
        match err {
            Error::Streaming(msg) => {
                assert!(msg.contains("AE"));
                assert!(msg.contains("PID-5 missing"));
            }
            other => panic!("expected Streaming, got {other:?}"),
        }
    }

    #[test]
    fn test_check_ack_rejects_ar() {
        let ack =
            "MSH|^~\\&|PAS|FAC|EMR|FAC|20260531120000||ACK|MSG3|P|2.5\rMSA|AR|MSG3|bad header\r";
        let err = Hl7v2MllpPublisher::cls_check_ack(ack).expect_err("AR must fail");
        assert!(matches!(err, Error::Streaming(ref s) if s.contains("AR")));
    }

    #[test]
    fn test_check_ack_missing_msa() {
        let ack = "MSH|^~\\&|PAS|FAC|EMR|FAC|20260531120000||ACK|MSG4|P|2.5\r";
        let err = Hl7v2MllpPublisher::cls_check_ack(ack).expect_err("missing MSA must fail");
        assert!(matches!(err, Error::Streaming(ref s) if s.contains("MSA")));
    }

    #[test]
    fn test_extract_uuid_handles_missing_and_bad_input() {
        let id = Uuid::new_v4();
        let good = json!({ "encounter_id": id.to_string(), "bed_id": "not-a-uuid" });
        assert_eq!(extract_uuid(&good, "encounter_id"), Some(id));
        assert_eq!(extract_uuid(&good, "bed_id"), None);
        assert_eq!(extract_uuid(&good, "absent"), None);
    }

    impl Hl7v2MllpPublisher {
        fn cls_check_ack(ack: &str) -> Result<()> {
            Self::check_ack(ack)
        }
    }
}

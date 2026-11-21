//! The outbound HL7 v2 publisher against a mock MLLP peer: each event of the outbox becomes the right ADT or
//! SIU frame, and the peer's answer decides whether delivery is done, retried or dead-lettered (NFR-32).
//! Needs PostgreSQL: `cargo test -- --ignored`.
mod common;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::hl7v2::mllp;
use patient_administration_system::streaming::{DomainEvent, EventPublisher, Hl7v2MllpPublisher};
use patient_administration_system::upstream::StubPersonClient;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

/// A peer that answers every frame with `reply` (an MSA code, `NONE` to close without answering, or `NOMSA`
/// for an ACK without an MSA segment) and records what it was sent.
async fn peer(reply: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (mut r, mut w) = stream.into_split();
            let Ok(Some(frame)) = mllp::read_frame(&mut r).await else {
                continue;
            };
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&frame).into_owned());
            if reply == "NONE" {
                continue;
            }
            let ack = if reply == "NOMSA" {
                "MSH|^~\\&|X|Y|PAS|HOSP|20261116||ACK|1|P|2.5\rERR|x\r".to_string()
            } else {
                format!("MSH|^~\\&|X|Y|PAS|HOSP|20261116||ACK|1|P|2.5\rMSA|{reply}|1\r")
            };
            let _ = mllp::write_frame(&mut w, ack.as_bytes()).await;
            let _ = w.shutdown().await;
        }
    });
    (addr, seen)
}

struct World {
    s: AppState,
    people: Arc<StubPersonClient>,
}

async fn world() -> World {
    let people = Arc::new(StubPersonClient::new());
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(people.clone());
    World { s, people }
}

async fn post(w: &World, path: &str, body: &str) -> String {
    let resp = patient_administration_system::api::rest::router(w.s.clone())
        .oneshot(
            Request::post(path)
                .header("content-type", "application/hl7-v2")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}

fn uuid_after(text: &str, key: &str) -> Uuid {
    let at = text
        .find(key)
        .unwrap_or_else(|| panic!("no {key} in {text:?}"))
        + key.len();
    Uuid::parse_str(&text[at..at + 36]).unwrap()
}

/// An outpatient encounter and a booked appointment for one person, made through the HL7 endpoints.
async fn encounter_and_appointment(w: &World) -> (Uuid, Uuid) {
    let tag = Uuid::new_v4().simple().to_string();
    let pid = format!("PID|1||MRN-{tag}^^^HOSP^MR||Lovelace^Ada||19900115|F\r");
    let head = |ev: &str| {
        format!(
            "MSH|^~\\&|EMR-{tag}|WARDS|PAS|HOSP|20261116120000||{ev}|{}|P|2.5\r",
            Uuid::new_v4().simple()
        )
    };
    let reg = post(
        w,
        "/api/hl7/v2/register",
        &format!("{}{pid}PV1|1|O\r", head("ADT^A04")),
    )
    .await;
    let enc = uuid_after(&reg, "encounter=");
    let book = post(
        w,
        "/api/hl7/v2/schedule-book",
        &format!(
            "{}SCH|P1||||||visit||30|min|^^^20281201093000\r{pid}",
            head("SIU^S12")
        ),
    )
    .await;
    assert!(book.contains("MSA|AA"), "{book:?}");
    let appt = uuid_after(&book, "filler=");
    (enc, appt)
}

fn event(kind: &str, key: &str, id: Uuid) -> DomainEvent {
    DomainEvent::new(kind, serde_json::json!({ key: id.to_string() }))
}

fn trigger_of(frame: &str) -> String {
    frame
        .split('\r')
        .next()
        .unwrap()
        .split('|')
        .nth(8)
        .unwrap()
        .to_string()
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn each_outbox_event_becomes_its_adt_or_siu_frame_and_an_aa_finishes_it() {
    let w = world().await;
    let (enc, appt) = encounter_and_appointment(&w).await;
    let (addr, seen) = peer("AA").await;
    let p = Hl7v2MllpPublisher::new(w.s.db.clone(), addr, "PAS", "EMR")
        .with_person_client(w.people.clone());
    let wanted = [
        ("EncounterRegistered", "encounter_id", enc, "ADT^A04"),
        ("EncounterAdmitted", "encounter_id", enc, "ADT^A01"),
        ("EncounterPreAdmitted", "encounter_id", enc, "ADT^A05"),
        ("EncounterPreAdmitCancelled", "encounter_id", enc, "ADT^A38"),
        (
            "EncounterPromotedToInpatient",
            "encounter_id",
            enc,
            "ADT^A06",
        ),
        (
            "EncounterDemotedToOutpatient",
            "encounter_id",
            enc,
            "ADT^A07",
        ),
        ("EncounterCancelled", "encounter_id", enc, "ADT^A11"),
        ("EncounterTransferred", "encounter_id", enc, "ADT^A02"),
        ("EncounterTransferCancelled", "encounter_id", enc, "ADT^A12"),
        ("EncounterDischarged", "encounter_id", enc, "ADT^A03"),
        (
            "EncounterDischargeCancelled",
            "encounter_id",
            enc,
            "ADT^A13",
        ),
        ("AppointmentBooked", "appointment_id", appt, "SIU^S12"),
        ("AppointmentRescheduled", "appointment_id", appt, "SIU^S13"),
        ("AppointmentModified", "appointment_id", appt, "SIU^S14"),
        ("AppointmentCancelled", "appointment_id", appt, "SIU^S15"),
        ("AppointmentNoShow", "appointment_id", appt, "SIU^S26"),
    ];
    for (kind, key, id, _) in &wanted {
        p.publish(event(kind, key, *id))
            .await
            .unwrap_or_else(|e| panic!("{kind}: {e}"));
    }
    let frames = seen.lock().unwrap().clone();
    let got: Vec<String> = frames.iter().map(|f| trigger_of(f)).collect();
    let want: Vec<String> = wanted.iter().map(|w| w.3.to_string()).collect();
    assert_eq!(got, want);

    // An event with no wire mapping is acknowledged without a connection.
    let before = seen.lock().unwrap().len();
    p.publish(DomainEvent::new("LetterGenerated", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), before);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_peers_answer_decides_between_done_retry_and_dead_letter() {
    let w = world().await;
    let (enc, _) = encounter_and_appointment(&w).await;
    let ev = || event("EncounterAdmitted", "encounter_id", enc);
    for (reply, expect) in [
        ("AA", None),
        ("AE", Some("Deferred")),
        ("AR", Some("Rejected")),
        ("AX", Some("unknown ACK code")),
        ("NOMSA", Some("missing MSA")),
        ("NONE", Some("closed before ACK")),
    ] {
        let (addr, _) = peer(reply).await;
        let p = Hl7v2MllpPublisher::new(w.s.db.clone(), addr, "PAS", "EMR")
            .with_person_client(w.people.clone());
        let r = p.publish(ev()).await;
        match expect {
            None => r.unwrap(),
            Some(text) => {
                let e = r.expect_err(reply);
                let shown = format!("{e:?} {e}");
                assert!(shown.contains(text), "{reply}: {shown}");
            }
        }
    }
    // Nobody listening.
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().to_string()
    };
    let p = Hl7v2MllpPublisher::new(w.s.db.clone(), closed, "PAS", "EMR")
        .with_person_client(w.people.clone());
    assert!(p.publish(ev()).await.is_err());
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn an_event_that_cannot_be_framed_fails_before_anything_is_sent() {
    let w = world().await;
    let (addr, seen) = peer("AA").await;
    let bare = Hl7v2MllpPublisher::new(w.s.db.clone(), addr.clone(), "PAS", "EMR");
    let with = Hl7v2MllpPublisher::new(w.s.db.clone(), addr, "PAS", "EMR")
        .with_person_client(w.people.clone())
        .with_worker_client(Arc::new(
            patient_administration_system::upstream::StubWorkerClient::new(),
        ));
    let id = Uuid::new_v4();
    // No person service to hydrate from.
    assert!(
        bare.publish(event("EncounterAdmitted", "encounter_id", id))
            .await
            .is_err()
    );
    assert!(
        bare.publish(event("AppointmentBooked", "appointment_id", id))
            .await
            .is_err()
    );
    // A payload without the id, an id that is nobody's, an id that is not a UUID.
    for kind in ["EncounterAdmitted", "AppointmentBooked"] {
        let none = DomainEvent::new(kind, serde_json::json!({}));
        assert!(with.publish(none).await.is_err(), "{kind} without an id");
        let bad = DomainEvent::new(
            kind,
            serde_json::json!({"encounter_id": "x", "appointment_id": "x"}),
        );
        assert!(with.publish(bad).await.is_err(), "{kind} with a bad id");
    }
    assert!(
        with.publish(event("EncounterAdmitted", "encounter_id", id))
            .await
            .is_err()
    );
    assert!(
        with.publish(event("AppointmentBooked", "appointment_id", id))
            .await
            .is_err()
    );
    assert!(seen.lock().unwrap().is_empty());
}

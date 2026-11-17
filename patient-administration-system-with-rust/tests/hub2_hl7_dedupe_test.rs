//! HUB-2, NFR-31: a resent HL7 v2 message is not processed twice. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! A sender that loses our ACK sends the same message again with the same MSH-10. These send an A01 twice, alone
//! and in a batch, and find one admission, one key and the same answer both times.
mod common;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::hl7v2::{SegmentRead, parse_message};
use patient_administration_system::upstream::StubPersonClient;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct World {
    s: AppState,
    sender: String,
    bed_code: String,
    mrn: String,
}

async fn world() -> World {
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let tag = Uuid::new_v4().simple().to_string()[..10].to_string();
    let (fac, ward, room, bed) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let bed_code = format!("BED-{tag}");
    for (sql, binds) in [
        (
            "INSERT INTO facilities (id, name, code, address, active, created_at, updated_at) VALUES ($1,$2,$3,'{}'::jsonb,true,now(),now())",
            vec![
                fac.into(),
                format!("hub2-{tag}").into(),
                format!("F{tag}").into(),
            ],
        ),
        (
            "INSERT INTO wards (id, facility_id, name, code, capacity, active, created_at, updated_at) VALUES ($1,$2,$3,$4,2,true,now(),now())",
            vec![
                ward.into(),
                fac.into(),
                format!("W{tag}").into(),
                format!("W{tag}").into(),
            ],
        ),
        (
            "INSERT INTO rooms (id, ward_id, name, code, active, created_at, updated_at) VALUES ($1,$2,$3,$4,true,now(),now())",
            vec![
                room.into(),
                ward.into(),
                "r".into(),
                format!("R{tag}").into(),
            ],
        ),
        (
            "INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1,$2,$3,$4,'available',now(),now())",
            vec![bed.into(), room.into(), "b".into(), bed_code.clone().into()],
        ),
    ] {
        s.db.run(Stmt::new(sql, binds)).await.unwrap();
    }
    World {
        s,
        sender: format!("EMR-{tag}"),
        bed_code,
        mrn: format!("MRN-{tag}"),
    }
}

fn a01(w: &World, control_id: &str) -> String {
    format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||ADT^A01|{control_id}|P|2.5\r\
         EVN|A01|20261116120000\r\
         PID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r\
         PV1|1|I|W^R^{}\r",
        w.sender, w.mrn, w.bed_code
    )
}

async fn post(w: &World, path: &str, body: &str) -> String {
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let resp = app
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

/// MSA-1 and MSA-2 of each ACK in `text` (one, or several in a batch).
fn msas(text: &str) -> Vec<(String, String)> {
    text.split('\r')
        .filter(|l| l.starts_with("MSA|"))
        .map(|l| {
            let m = parse_message(&format!("MSH|^~\\&|X\r{l}\r")).unwrap();
            let s = m.segment("MSA").unwrap();
            (s.raw(1), s.raw(2))
        })
        .collect()
}

async fn admissions(w: &World) -> i64 {
    let row = w
        .s
        .db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM admissions a JOIN beds b ON b.id = a.bed_id WHERE b.code = $1",
            [w.bed_code.clone().into()],
        ))
        .await
        .unwrap();
    row as i64
}

fn sha(control_id: &str) -> String {
    Sha256::digest(control_id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn key_state(w: &World, control_id: &str) -> Option<String> {
    use patient_administration_system::db::row::RowExt;
    w.s.db
        .one(Stmt::new(
            "SELECT state FROM hl7_inbound_messages WHERE sending_app = $1 AND sending_facility = 'WARDS' AND control_id_sha256 = $2",
            [w.sender.clone().into(), sha(control_id).into()],
        ))
        .await
        .unwrap()
        .map(|r| r.col::<String>("state").unwrap())
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_same_admission_sent_twice_admits_once_and_answers_alike() {
    let w = world().await;
    let id = format!("MSG-{}", Uuid::new_v4().simple());
    let first = post(&w, "/api/hl7/v2/admit", &a01(&w, &id)).await;
    let again = post(&w, "/api/hl7/v2/admit", &a01(&w, &id)).await;
    assert_eq!(
        msas(&first),
        vec![("AA".to_string(), id.clone())],
        "{first}"
    );
    assert_eq!(msas(&again), msas(&first), "{again}");
    let text = |a: &str| {
        a.split('\r')
            .find(|l| l.starts_with("MSA|"))
            .unwrap()
            .to_string()
    };
    assert_eq!(
        text(&again),
        text(&first),
        "the resend gets the first answer's text"
    );
    assert_eq!(admissions(&w).await, 1, "one admission, not two");
    assert_eq!(key_state(&w, &id).await.as_deref(), Some("done"));
    // Only a hash of the id is kept.
    let raw = w
        .s
        .db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM hl7_inbound_messages WHERE control_id_sha256 = $1 OR ack_text LIKE '%' || $1 || '%'",
            [id.clone().into()],
        ))
        .await
        .unwrap();
    assert_eq!(raw, 0);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_different_control_id_is_processed() {
    let w = world().await;
    let first = post(
        &w,
        "/api/hl7/v2/admit",
        &a01(&w, &format!("A-{}", Uuid::new_v4().simple())),
    )
    .await;
    assert_eq!(msas(&first)[0].0, "AA");
    // A new message for the same patient is processed: the patient is already admitted, so it is an error, not
    // a replay of the first AA.
    let other = format!("B-{}", Uuid::new_v4().simple());
    let second = post(&w, "/api/hl7/v2/admit", &a01(&w, &other)).await;
    assert_eq!(
        msas(&second),
        vec![("AE".to_string(), other.clone())],
        "{second}"
    );
    assert_eq!(admissions(&w).await, 1);
    // An error changes nothing, so its key is freed for a corrected resend.
    assert_eq!(key_state(&w, &other).await, None);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_copy_that_has_not_finished_is_refused_not_processed() {
    let w = world().await;
    let id = format!("MSG-{}", Uuid::new_v4().simple());
    w.s.db
        .run(Stmt::new(
            "INSERT INTO hl7_inbound_messages (sending_app, sending_facility, control_id_sha256, state) VALUES ($1, 'WARDS', $2, 'processing')",
            [w.sender.clone().into(), sha(&id).into()],
        ))
        .await
        .unwrap();
    let ack = post(&w, "/api/hl7/v2/admit", &a01(&w, &id)).await;
    assert_eq!(msas(&ack), vec![("AE".to_string(), id.clone())], "{ack}");
    assert!(ack.contains("earlier copy has not finished"), "{ack}");
    assert_eq!(admissions(&w).await, 0, "nothing processed");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_same_message_twice_in_a_batch_admits_once() {
    let w = world().await;
    let id = format!("MSG-{}", Uuid::new_v4().simple());
    let one = a01(&w, &id);
    let batch = format!(
        "BHS|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||||B-1\r{one}{one}BTS|2\r",
        w.sender
    );
    let ack = post(&w, "/api/hl7/v2/batch", &batch).await;
    assert_eq!(
        msas(&ack),
        vec![
            ("AA".to_string(), id.clone()),
            ("AA".to_string(), id.clone())
        ],
        "{ack}"
    );
    assert_eq!(admissions(&w).await, 1);
    // And the same message alone, later, is answered from the batch's first ACK.
    let alone = post(&w, "/api/hl7/v2/admit", &one).await;
    assert_eq!(msas(&alone)[0].0, "AA");
    assert_eq!(admissions(&w).await, 1);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn parse_changes_nothing_so_it_is_never_recorded() {
    let w = world().await;
    let id = format!("MSG-{}", Uuid::new_v4().simple());
    post(&w, "/api/hl7/v2/parse", &a01(&w, &id)).await;
    assert_eq!(key_state(&w, &id).await, None);
}

//! DEV-4, audit PA-16: an HL7 v2 message posted to the HTTP path of another type is refused and changes
//! nothing. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! An admission posted to the discharge path is answered AR twice (the refusal is not remembered), then the
//! same message, same control id, posted to the admission path admits once. An admission over MLLP and one over
//! HTTP have the same outcome.
mod common;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::hl7v2::{SegmentRead, parse_message};
use patient_administration_system::upstream::StubPersonClient;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use uuid::Uuid;

struct World {
    s: AppState,
    sender: String,
    beds: Vec<String>,
}

async fn world() -> World {
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let tag = Uuid::new_v4().simple().to_string()[..10].to_string();
    let (fac, ward, room) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let beds: Vec<String> = (0..2).map(|i| format!("BED{i}-{tag}")).collect();
    let mut stmts = vec![
        (
            "INSERT INTO facilities (id, name, code, address, active, created_at, updated_at) VALUES ($1,$2,$3,'{}'::jsonb,true,now(),now())",
            vec![
                fac.into(),
                format!("dev4-{tag}").into(),
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
    ];
    for code in &beds {
        stmts.push((
            "INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1,$2,$3,$4,'available',now(),now())",
            vec![Uuid::new_v4().into(), room.into(), "b".into(), code.clone().into()],
        ));
    }
    for (sql, binds) in stmts {
        s.db.run(Stmt::new(sql, binds)).await.unwrap();
    }
    World {
        s,
        sender: format!("EMR-{tag}"),
        beds,
    }
}

fn a01(w: &World, bed: &str, control_id: &str) -> String {
    format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261128120000||ADT^A01^ADT_A01|{control_id}|P|2.5\r\
         EVN|A01|20261128120000\r\
         PID|1||MRN-{control_id}^^^HOSP^MR||Lovelace^Ada||19900115|F\r\
         PV1|1|I|W^R^{bed}\r",
        w.sender
    )
}

fn oru(w: &World, control_id: &str) -> String {
    format!(
        "MSH|^~\\&|{}|LAB|PAS|HOSP|20261128120000||ORU^R01|{control_id}|P|2.5\r\
         PID|1||MRN-{control_id}^^^HOSP^MR||Lovelace^Ada||19900115|F\r\
         PV1|1|I|W^R^{}\r\
         OBR|1\r",
        w.sender, w.beds[0]
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

async fn mllp(w: &World, body: &str) -> String {
    let (mut client, mut server) = tokio::io::duplex(1 << 20);
    let router = patient_administration_system::api::rest::router(w.s.clone());
    let frame = [&[0x0b][..], body.as_bytes(), &[0x1c, 0x0d][..]].concat();
    client.write_all(&frame).await.unwrap();
    patient_administration_system::hl7v2::listener::handle_one_frame(&mut server, router)
        .await
        .unwrap();
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    String::from_utf8(out)
        .unwrap()
        .trim_matches(['\u{b}', '\u{1c}', '\r'])
        .to_string()
}

/// MSA-1, MSA-2 and MSA-3 of the ACK, MSA-3 unescaped (`^` travels as `\\S\\`).
fn msa(ack: &str) -> (String, String, String) {
    let m = parse_message(ack).unwrap();
    let s = m
        .segment("MSA")
        .unwrap_or_else(|| panic!("no MSA in {ack}"));
    (s.raw(1), s.raw(2), s.raw(3).replace("\\S\\", "^"))
}

async fn admissions(w: &World, bed: &str) -> i64 {
    w.s.db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM admissions a JOIN beds b ON b.id = a.bed_id WHERE b.code = $1",
            [bed.to_string().into()],
        ))
        .await
        .unwrap() as i64
}

async fn remembered(w: &World) -> i64 {
    w.s.db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM hl7_inbound_messages WHERE sending_app = $1",
            [w.sender.clone().into()],
        ))
        .await
        .unwrap() as i64
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_message_at_the_wrong_path_is_refused_twice_then_admits_at_the_right_one() {
    let w = world().await;
    let id = format!("MSG-{}", Uuid::new_v4().simple());
    let msg = a01(&w, &w.beds[0], &id);
    let refused = "MSH-9 does not match this path, which takes ADT^A03".to_string();
    for _ in 0..2 {
        let ack = post(&w, "/api/hl7/v2/discharge", &msg).await;
        assert_eq!(
            msa(&ack),
            ("AR".into(), id.clone(), refused.clone()),
            "{ack}"
        );
        assert!(
            !ack.contains("Lovelace") && !ack.contains("MRN-"),
            "no patient data: {ack}"
        );
    }
    assert_eq!(
        admissions(&w, &w.beds[0]).await,
        0,
        "a refusal changes nothing"
    );
    assert_eq!(remembered(&w).await, 0, "a refusal is not remembered");

    let ack = post(&w, "/api/hl7/v2/admit", &msg).await;
    assert_eq!(msa(&ack).0, "AA", "{ack}");
    assert_eq!(admissions(&w, &w.beds[0]).await, 1);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn an_unhandled_type_posted_as_an_admission_changes_nothing() {
    let w = world().await;
    let id = format!("LAB-{}", Uuid::new_v4().simple());
    let ack = post(&w, "/api/hl7/v2/admit", &oru(&w, &id)).await;
    assert_eq!(
        msa(&ack),
        (
            "AR".into(),
            id,
            "MSH-9 does not match this path, which takes ADT^A01".into()
        ),
        "{ack}"
    );
    assert_eq!(admissions(&w, &w.beds[0]).await, 0);
    assert_eq!(remembered(&w).await, 0);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn an_admission_over_mllp_and_over_http_have_the_same_outcome() {
    let w = world().await;
    let (by_http, by_mllp) = (
        format!("H-{}", Uuid::new_v4().simple()),
        format!("M-{}", Uuid::new_v4().simple()),
    );
    let h = post(&w, "/api/hl7/v2/admit", &a01(&w, &w.beds[0], &by_http)).await;
    let m = mllp(&w, &a01(&w, &w.beds[1], &by_mllp)).await;
    assert_eq!(msa(&h).0, "AA", "{h}");
    assert_eq!(msa(&m).0, "AA", "{m}");
    assert_eq!(admissions(&w, &w.beds[0]).await, 1);
    assert_eq!(admissions(&w, &w.beds[1]).await, 1);
    // An unhandled type over MLLP is refused too, and changes nothing.
    let lab = mllp(&w, &oru(&w, &format!("L-{}", Uuid::new_v4().simple()))).await;
    assert_eq!(msa(&lab).0, "AR", "{lab}");
    assert_eq!(admissions(&w, &w.beds[0]).await, 1);
}

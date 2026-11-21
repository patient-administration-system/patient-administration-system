//! The HL7 v2 "change" endpoints (pre-admit, cancel, leave, update, delete, merge) answer every message with an
//! ACK, and answer `AE` rather than failing when the message lacks what the change needs. Needs PostgreSQL:
//! `cargo test -- --ignored`. The admit path and the resend rules are in `hub2_hl7_dedupe_test.rs`.
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
use tower::ServiceExt;
use uuid::Uuid;

const CHANGES: [(&str, &str); 12] = [
    ("pre-admit", "A05"),
    ("cancel-pre-admit", "A38"),
    ("change-to-inpatient", "A06"),
    ("change-to-outpatient", "A07"),
    ("update", "A08"),
    ("cancel-admit", "A11"),
    ("cancel-transfer", "A12"),
    ("cancel-discharge", "A13"),
    ("leave-start", "A21"),
    ("leave-end", "A22"),
    ("delete-patient", "A23"),
    ("merge", "A40"),
];

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
                format!("chg-{tag}").into(),
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

/// A message of `event`, with or without a PID and a bed.
fn message(w: &World, event: &str, pid: bool, bed: bool) -> String {
    let mut m = format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||ADT^{event}|{}|P|2.5\rEVN|{event}|20261116120000\r",
        w.sender,
        Uuid::new_v4().simple()
    );
    if pid {
        m += &format!("PID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r", w.mrn);
    }
    if event == "A40" {
        m += &format!("MRG|OLD-{}^^^HOSP^MR\r", w.mrn);
    }
    if bed {
        m += &format!("PV1|1|I|W^R^{}\r", w.bed_code);
    }
    m
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

/// MSA-1 of the ACK in `text`, or the text itself when there is none.
fn code(text: &str) -> String {
    let line = text
        .split('\r')
        .find(|l| l.starts_with("MSA|"))
        .unwrap_or(text);
    parse_message(&format!("MSH|^~\\&|X\r{line}\r"))
        .ok()
        .and_then(|m| m.segment("MSA").map(|s| s.raw(1)))
        .unwrap_or_else(|| text.to_string())
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn every_change_endpoint_acknowledges_a_whole_message_and_a_bare_one() {
    let w = world().await;
    for (path, event) in CHANGES {
        let url = format!("/api/hl7/v2/{path}");
        for (pid, bed) in [(true, true), (true, false), (false, false)] {
            let ack = post(&w, &url, &message(&w, event, pid, bed)).await;
            let c = code(&ack);
            assert!(
                ["AA", "AE", "AR", "CA", "CE", "CR"].contains(&c.as_str()),
                "{path} pid={pid} bed={bed}: {ack:?}"
            );
        }
        // Not HL7 at all: still an answer, never an empty one.
        assert!(!post(&w, &url, "not hl7").await.is_empty(), "{path}");
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_pre_admission_needs_a_bed_that_exists() {
    let w = world().await;
    let none = post(
        &w,
        "/api/hl7/v2/pre-admit",
        &message(&w, "A05", true, false),
    )
    .await;
    assert_eq!(code(&none), "AE", "{none:?}");
    let mut unknown = message(&w, "A05", true, false);
    unknown += "PV1|1|I|W^R^NO-SUCH-BED\r";
    let ack = post(&w, "/api/hl7/v2/pre-admit", &unknown).await;
    assert_eq!(code(&ack), "AE", "{ack:?}");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_pre_admission_is_taken_and_can_be_cancelled() {
    let w = world().await;
    let ok = post(&w, "/api/hl7/v2/pre-admit", &message(&w, "A05", true, true)).await;
    assert_eq!(code(&ok), "AA", "{ok:?}");
    let cancel = post(
        &w,
        "/api/hl7/v2/cancel-pre-admit",
        &message(&w, "A38", true, true),
    )
    .await;
    assert!(["AA", "AE"].contains(&code(&cancel).as_str()), "{cancel:?}");
}

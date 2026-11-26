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
    room: Uuid,
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
        room,
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

/// Another available bed in the world's room.
async fn another_bed(w: &World) -> String {
    let code = format!("BED-{}", &Uuid::new_v4().simple().to_string()[..10]);
    w.s.db
        .run(Stmt::new(
            "INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1,$2,$3,$4,'available',now(),now())",
            vec![Uuid::new_v4().into(), w.room.into(), "b".into(), code.clone().into()],
        ))
        .await
        .unwrap();
    code
}

/// `event` for the world's person in `bed`.
fn in_bed(w: &World, event: &str, bed: &str) -> String {
    let mut m = message(w, event, true, false);
    m += &format!("PV1|1|I|W^R^{bed}\r");
    m
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn an_admission_can_be_left_updated_moved_discharged_and_taken_back() {
    let w = world().await;
    let b2 = another_bed(&w).await;
    let steps: [(&str, &str, &str, &str); 9] = [
        ("admit", "A01", &w.bed_code, "AA"),
        ("leave-start", "A21", &w.bed_code, "AA"),
        ("leave-end", "A22", &w.bed_code, "AA"),
        ("update", "A08", &w.bed_code, "AA"),
        ("transfer", "A02", &b2, "AA"),
        ("cancel-transfer", "A12", &b2, "AA"),
        ("discharge", "A03", &b2, "AA"),
        ("cancel-discharge", "A13", &b2, "AA"),
        ("cancel-admit", "A11", &b2, "AA"),
    ];
    for (path, event, bed, want) in steps {
        let ack = post(&w, &format!("/api/hl7/v2/{path}"), &in_bed(&w, event, bed)).await;
        assert_eq!(code(&ack), want, "{path}: {ack:?}");
    }
    // Nothing is open any more.
    for (path, event) in [
        ("cancel-admit", "A11"),
        ("leave-start", "A21"),
        ("change-to-outpatient", "A07"),
    ] {
        let ack = post(
            &w,
            &format!("/api/hl7/v2/{path}"),
            &in_bed(&w, event, &w.bed_code),
        )
        .await;
        assert_eq!(code(&ack), "AE", "{path} with no open admission: {ack:?}");
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_pre_admission_is_cancelled_and_an_outpatient_is_promoted_and_returned() {
    let w = world().await;
    let bed = another_bed(&w).await;
    let pre = post(&w, "/api/hl7/v2/pre-admit", &in_bed(&w, "A05", &bed)).await;
    assert_eq!(code(&pre), "AA", "{pre:?}");
    let cancel = post(&w, "/api/hl7/v2/cancel-pre-admit", &in_bed(&w, "A38", &bed)).await;
    assert_eq!(code(&cancel), "AA", "{cancel:?}");

    // Promoting needs an outpatient encounter to promote.
    let early = post(
        &w,
        "/api/hl7/v2/change-to-inpatient",
        &in_bed(&w, "A06", &bed),
    )
    .await;
    assert_eq!(code(&early), "AE", "no outpatient yet: {early:?}");
    let mut reg = message(&w, "A04", true, false);
    reg += "PV1|1|O\r";
    let reg = post(&w, "/api/hl7/v2/register", &reg).await;
    assert_eq!(code(&reg), "AA", "{reg:?}");
    let inpatient = post(
        &w,
        "/api/hl7/v2/change-to-inpatient",
        &in_bed(&w, "A06", &bed),
    )
    .await;
    assert_eq!(code(&inpatient), "AA", "{inpatient:?}");
    let outpatient = post(
        &w,
        "/api/hl7/v2/change-to-outpatient",
        &in_bed(&w, "A07", &bed),
    )
    .await;
    assert_eq!(code(&outpatient), "AA", "{outpatient:?}");
    for path in ["change-to-inpatient", "pre-admit", "cancel-pre-admit"] {
        let none = post(
            &w,
            &format!("/api/hl7/v2/{path}"),
            &message(&w, "A06", true, false),
        )
        .await;
        assert_eq!(code(&none), "AE", "{path} without a bed: {none:?}");
        let unknown = post(
            &w,
            &format!("/api/hl7/v2/{path}"),
            &in_bed(&w, "A06", "NO-SUCH-BED"),
        )
        .await;
        assert_eq!(code(&unknown), "AE", "{path} unknown bed: {unknown:?}");
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn deleting_a_patient_and_merging_two_records() {
    let w = world().await;
    // The record to merge away exists under its own MRN.
    let old = format!("OLD-{}", w.mrn);
    let old_pid = format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||ADT^A04|{}|P|2.5\rEVN|A04|20261116120000\rPID|1||{old}^^^HOSP^MR||Lovelace^Ada||19900115|F\rPV1|1|O\r",
        w.sender,
        Uuid::new_v4().simple()
    );
    let reg = post(&w, "/api/hl7/v2/register", &old_pid).await;
    assert_eq!(code(&reg), "AA", "{reg:?}");
    let merge = post(&w, "/api/hl7/v2/merge", &message(&w, "A40", true, false)).await;
    assert!(["AA", "AE"].contains(&code(&merge).as_str()), "{merge:?}");
    let gone = post(
        &w,
        "/api/hl7/v2/delete-patient",
        &message(&w, "A23", true, false),
    )
    .await;
    assert_eq!(code(&gone), "AA", "{gone:?}");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_basic_adt_endpoints_refuse_what_they_cannot_act_on() {
    let w = world().await;
    let head = |event: &str| {
        format!(
            "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||ADT^{event}|{}|P|2.5\r",
            w.sender,
            Uuid::new_v4().simple()
        )
    };
    let pid = format!("PID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r", w.mrn);
    for (path, event) in [
        ("patient", "A28"),
        ("admit", "A01"),
        ("transfer", "A02"),
        ("discharge", "A03"),
        ("register", "A04"),
    ] {
        let url = format!("/api/hl7/v2/{path}");
        // Not HL7 at all: rejected, not accepted.
        let junk = post(&w, &url, "not hl7").await;
        assert_eq!(code(&junk), "AR", "{path}: {junk:?}");
        // No PID.
        let no_pid = post(
            &w,
            &url,
            &format!("{}PV1|1|I|W^R^{}\r", head(event), w.bed_code),
        )
        .await;
        assert_eq!(code(&no_pid), "AE", "{path} without PID: {no_pid:?}");
    }
    for (path, event) in [("admit", "A01"), ("transfer", "A02")] {
        let url = format!("/api/hl7/v2/{path}");
        // No PV1 where a bed is needed (a registration defaults to an outpatient).
        let no_pv1 = post(&w, &url, &format!("{}{pid}", head(event))).await;
        assert_eq!(code(&no_pv1), "AE", "{path} without PV1: {no_pv1:?}");
    }
    for (path, event) in [("admit", "A01"), ("transfer", "A02")] {
        let url = format!("/api/hl7/v2/{path}");
        let no_bed = post(&w, &url, &format!("{}{pid}PV1|1|I\r", head(event))).await;
        assert_eq!(code(&no_bed), "AE", "{path} without a bed: {no_bed:?}");
        let unknown = post(
            &w,
            &url,
            &format!("{}{pid}PV1|1|I|W^R^NO-SUCH-BED\r", head(event)),
        )
        .await;
        assert_eq!(
            code(&unknown),
            "AE",
            "{path} with an unknown bed: {unknown:?}"
        );
    }
    // A new person by A28 is created, then matched the second time.
    let first = post(&w, "/api/hl7/v2/patient", &format!("{}{pid}", head("A28"))).await;
    assert_eq!(code(&first), "AA", "{first:?}");
    let second = post(&w, "/api/hl7/v2/patient", &format!("{}{pid}", head("A28"))).await;
    assert_eq!(code(&second), "AA", "{second:?}");
    assert!(second.contains("matched existing"), "{second:?}");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn without_a_person_service_the_endpoints_say_so() {
    let mut w = world().await;
    // A state with no person client: nothing can be resolved from a PID.
    w.s = common::build_state(&common::database_url().unwrap()).await;
    let pid = format!("PID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r", w.mrn);
    let msg = |event: &str| {
        format!(
            "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||ADT^{event}|{}|P|2.5\r{pid}PV1|1|I|W^R^{}\r",
            w.sender,
            Uuid::new_v4().simple(),
            w.bed_code
        )
    };
    // Every endpoint that resolves a PID, the movements and each change.
    let movements = [
        ("patient", "A28"),
        ("admit", "A01"),
        ("transfer", "A02"),
        ("discharge", "A03"),
        ("register", "A04"),
    ];
    for (path, event) in movements.iter().chain(CHANGES.iter()) {
        let ack = post(&w, &format!("/api/hl7/v2/{path}"), &msg(event)).await;
        assert_eq!(code(&ack), "AE", "{path}: {ack:?}");
        assert!(
            ack.contains("PERSON_SERVICE_URL") || ack.contains("person"),
            "{path}: {ack:?}"
        );
    }
}

/// A batch routes each message by its type (MSH-9) to the same handler a lone message would reach, answers
/// each with its own ACK in order, and refuses a type the PAS does not handle.
#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_batch_routes_every_supported_type_and_refuses_the_rest() {
    let w = world().await;
    let types = [
        ("ADT", "A01"),
        ("ADT", "A02"),
        ("ADT", "A03"),
        ("ADT", "A04"),
        ("ADT", "A05"),
        ("ADT", "A06"),
        ("ADT", "A07"),
        ("ADT", "A08"),
        ("ADT", "A11"),
        ("ADT", "A12"),
        ("ADT", "A13"),
        ("ADT", "A21"),
        ("ADT", "A22"),
        ("ADT", "A23"),
        ("ADT", "A28"),
        ("ADT", "A38"),
        ("ADT", "A40"),
        ("DFT", "P03"),
        ("SIU", "S12"),
        ("SIU", "S13"),
        ("SIU", "S14"),
        ("SIU", "S15"),
        ("MFN", "M02"),
        ("MFN", "M05"),
        ("ORU", "R01"),
    ];
    let mut batch = format!(
        "BHS|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||||B-{}\r",
        w.sender,
        Uuid::new_v4().simple()
    );
    for (code, event) in types {
        batch += &format!(
            "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||{code}^{event}|{}|P|2.5\rPID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\rPV1|1|I|W^R^{}\r",
            w.sender,
            Uuid::new_v4().simple(),
            w.mrn,
            w.bed_code
        );
    }
    batch += &format!("BTS|{}\r", types.len());
    let ack = post(&w, "/api/hl7/v2/batch", &batch).await;
    let codes: Vec<&str> = ack
        .split('\r')
        .filter(|l| l.starts_with("MSA|"))
        .map(|l| l.split('|').nth(1).unwrap_or(""))
        .collect();
    assert_eq!(codes.len(), types.len(), "one ACK per message: {ack:?}");
    assert!(codes.iter().all(|c| !c.is_empty()), "{codes:?}");
    assert_eq!(
        codes.last(),
        Some(&"AR"),
        "an unsupported type is refused: {codes:?}"
    );
    // A batch that is not one is refused as a whole.
    assert!(!post(&w, "/api/hl7/v2/batch", "not hl7").await.is_empty());
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn every_change_endpoint_refuses_a_patient_it_has_nothing_to_change_for() {
    let w = world().await;
    // A known person with no admission, no pre-admission and no leave.
    let created = post(&w, "/api/hl7/v2/patient", &message(&w, "A28", true, false)).await;
    assert_eq!(code(&created), "AA", "{created:?}");
    for (path, event) in [
        ("cancel-pre-admit", "A38"),
        ("change-to-outpatient", "A07"),
        ("cancel-admit", "A11"),
        ("cancel-transfer", "A12"),
        ("cancel-discharge", "A13"),
        ("leave-start", "A21"),
        ("leave-end", "A22"),
    ] {
        let url = format!("/api/hl7/v2/{path}");
        let ack = post(&w, &url, &message(&w, event, true, true)).await;
        assert_eq!(code(&ack), "AE", "{path}: {ack:?}");
        // The refusal says why, in words fit for the sender.
        assert!(ack.contains("MSA|AE|"), "{path}: {ack:?}");
    }
    // A merge names a source number in MRG-1; with none it is refused before anything is read.
    let no_mrg = post(
        &w,
        "/api/hl7/v2/merge",
        &message(&w, "A40", true, false).replace("MRG|", "XXX|"),
    )
    .await;
    assert_eq!(code(&no_mrg), "AE", "{no_mrg:?}");
}

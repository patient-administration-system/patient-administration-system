//! HL7 v2 DFT^P03 (charges): each FT1 becomes a charge on the patient's open account, which is opened when
//! there is none; a message the PAS cannot post is answered `AE` and posts nothing. Needs PostgreSQL:
//! `cargo test -- --ignored`.
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

struct World {
    s: AppState,
    sender: String,
    mrn: String,
}

async fn world() -> World {
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let tag = Uuid::new_v4().simple().to_string()[..10].to_string();
    World {
        s,
        sender: format!("BILL-{tag}"),
        mrn: format!("MRN-{tag}"),
    }
}

fn dft(w: &World, ft1: &[&str]) -> String {
    let mut m = format!(
        "MSH|^~\\&|{}|FAC|PAS|FAC|20261116120000||DFT^P03|{}|P|2.5\rEVN|P03|20261116120000\rPID|1||{}^^^FAC^MR||Doe^Jane||19900115|F\r",
        w.sender,
        Uuid::new_v4().simple(),
        w.mrn
    );
    for f in ft1 {
        m += f;
        m += "\r";
    }
    m
}

async fn post(w: &World, body: &str) -> String {
    let resp = patient_administration_system::api::rest::router(w.s.clone())
        .oneshot(
            Request::post("/api/hl7/v2/dft")
                .header("content-type", "application/hl7-v2")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}

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

async fn charges_for(w: &World, code: &str) -> i64 {
    w.s.db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM charges WHERE code = $1",
            [code.to_string().into()],
        ))
        .await
        .unwrap() as i64
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn charges_are_posted_to_one_account_that_is_opened_once() {
    let w = world().await;
    let c1 = format!("C1-{}", w.mrn);
    let c2 = format!("C2-{}", w.mrn);
    let one = post(
        &w,
        &dft(
            &w,
            &[&format!(
                "FT1|1|||20261116100000||CG|{c1}^Consult|Outpatient consultation|||25.00^GBP"
            )],
        ),
    )
    .await;
    assert_eq!(code(&one), "AA", "{one:?}");
    assert!(one.contains("charge posted account="), "{one:?}");
    let account = one.split("account=").nth(1).unwrap()[..36].to_string();

    // Two more on the same account: the open account is reused, not opened again.
    let two = post(
        &w,
        &dft(
            &w,
            &[
                &format!("FT1|1|||||CG|{c1}^Consult|Follow-up|||10.50^GBP"),
                &format!("FT1|2|||20261116110000||CG|{c2}^Dressing|Dressing change|||4.00^GBP"),
            ],
        ),
    )
    .await;
    assert_eq!(code(&two), "AA", "{two:?}");
    assert!(two.contains("charges_posted=2"), "{two:?}");
    assert!(two.contains(&account), "the same account: {two:?}");
    assert_eq!(charges_for(&w, &c1).await, 2);
    assert_eq!(charges_for(&w, &c2).await, 1);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_message_that_cannot_be_posted_is_refused_and_posts_nothing() {
    let w = world().await;
    let good = "FT1|1|||||CG|OK1^Consult|Consultation|||5.00^GBP";
    let cases: [(&str, Vec<String>); 8] = [
        ("no FT1 at all", vec![]),
        (
            "a reversal, not a charge",
            vec!["FT1|1|||||CD|OK1^Consult|Consultation|||5.00^GBP".into()],
        ),
        (
            "two currencies",
            vec![
                good.into(),
                "FT1|2|||||CG|OK2^Consult|Other|||5.00^USD".into(),
            ],
        ),
        (
            "an amount that is not a number",
            vec!["FT1|1|||||CG|OK1^Consult|Consultation|||lots^GBP".into()],
        ),
        (
            "a currency that is not ISO 4217",
            vec!["FT1|1|||||CG|OK1^Consult|Consultation|||5.00^pounds".into()],
        ),
        (
            "no code",
            vec!["FT1|1|||||CG||Consultation|||5.00^GBP".into()],
        ),
        (
            "no description",
            vec!["FT1|1|||||CG|OK1^Consult||||5.00^GBP".into()],
        ),
        (
            "no amount",
            vec!["FT1|1|||||CG|OK1^Consult|Consultation|||^GBP".into()],
        ),
    ];
    for (what, ft1) in cases {
        let refs: Vec<&str> = ft1.iter().map(String::as_str).collect();
        let ack = post(&w, &dft(&w, &refs)).await;
        assert_eq!(code(&ack), "AE", "{what}: {ack:?}");
    }
    assert_eq!(charges_for(&w, "OK1").await, 0);
    // A message with no PID is answered too.
    let no_pid = format!(
        "MSH|^~\\&|{}|FAC|PAS|FAC|20261116120000||DFT^P03|{}|P|2.5\r{good}\r",
        w.sender,
        Uuid::new_v4().simple()
    );
    assert_eq!(code(&post(&w, &no_pid).await), "AE");
}

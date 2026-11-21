//! The HL7 v2 scheduling (SIU) and master-file (MFN) endpoints: a booking is taken, moved, changed and
//! cancelled by message, and a message that lacks what the change needs is answered `AE`. Needs PostgreSQL:
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
        mrn: format!("MRN-{tag}"),
    }
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

/// A scheduling message; `filler` is the appointment id, `start` the SCH-11 time.
fn siu(w: &World, event: &str, filler: Option<&str>, start: Option<&str>) -> String {
    format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||SIU^{event}|{}|P|2.5\rSCH|P1|{}|||||visit||30|min|{}\rPID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r",
        w.sender,
        Uuid::new_v4().simple(),
        filler.unwrap_or(""),
        start.map_or(String::new(), |s| format!("^^^{s}")),
        w.mrn
    )
}

/// The first UUID in `text`.
fn uuid_in(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(35)).find_map(|i| {
        let s = text.get(i..i + 36)?;
        Uuid::parse_str(s).ok().map(|u| u.to_string())
    })
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_booking_is_taken_moved_changed_and_cancelled_by_message() {
    let w = world().await;
    let book = post(
        &w,
        "/api/hl7/v2/schedule-book",
        &siu(&w, "S12", None, Some("20271201093000")),
    )
    .await;
    assert_eq!(code(&book), "AA", "{book:?}");
    let id = uuid_in(&book).expect("the ACK names the appointment");

    // The same person cannot be in two places at once.
    let clash = post(
        &w,
        "/api/hl7/v2/schedule-book",
        &siu(&w, "S12", None, Some("20271201094000")),
    )
    .await;
    assert_eq!(code(&clash), "AE", "{clash:?}");

    let moved = post(
        &w,
        "/api/hl7/v2/schedule-reschedule",
        &siu(&w, "S13", Some(&id), Some("20271202093000")),
    )
    .await;
    assert_eq!(code(&moved), "AA", "{moved:?}");
    let changed = post(
        &w,
        "/api/hl7/v2/schedule-modify",
        &siu(&w, "S14", Some(&id), Some("20271202100000")),
    )
    .await;
    assert!(
        ["AA", "AE"].contains(&code(&changed).as_str()),
        "{changed:?}"
    );
    let cancelled = post(
        &w,
        "/api/hl7/v2/schedule-cancel",
        &siu(&w, "S15", Some(&id), None),
    )
    .await;
    assert_eq!(code(&cancelled), "AA", "{cancelled:?}");
    let again = post(
        &w,
        "/api/hl7/v2/schedule-cancel",
        &siu(&w, "S15", Some(&id), None),
    )
    .await;
    assert!(["AA", "AE"].contains(&code(&again).as_str()), "{again:?}");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_scheduling_message_without_what_it_needs_is_refused_with_an_ae() {
    let w = world().await;
    let gone = Uuid::new_v4().to_string();
    // No SCH segment at all.
    let no_sch = format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||SIU^S12|{}|P|2.5\rPID|1||{}^^^HOSP^MR||Lovelace^Ada||19900115|F\r",
        w.sender,
        Uuid::new_v4().simple(),
        w.mrn
    );
    for path in [
        "schedule-book",
        "schedule-reschedule",
        "schedule-modify",
        "schedule-cancel",
    ] {
        let url = format!("/api/hl7/v2/{path}");
        let ack = post(&w, &url, &no_sch).await;
        assert_eq!(code(&ack), "AE", "{path} without SCH: {ack:?}");
    }
    // No start time.
    let ack = post(&w, "/api/hl7/v2/schedule-book", &siu(&w, "S12", None, None)).await;
    assert_eq!(code(&ack), "AE", "{ack:?}");
    // No filler id, and a filler id that is nobody's.
    for path in ["schedule-reschedule", "schedule-modify", "schedule-cancel"] {
        let url = format!("/api/hl7/v2/{path}");
        let none = post(&w, &url, &siu(&w, "S13", None, Some("20271203093000"))).await;
        assert_eq!(code(&none), "AE", "{path} without a filler id: {none:?}");
        let unknown = post(
            &w,
            &url,
            &siu(&w, "S13", Some(&gone), Some("20271203093000")),
        )
        .await;
        assert_eq!(
            code(&unknown),
            "AE",
            "{path} with an unknown appointment: {unknown:?}"
        );
    }
    // Not HL7: an answer all the same.
    for path in ["schedule-book", "mfn-staff", "mfn-location"] {
        assert!(
            !post(&w, &format!("/api/hl7/v2/{path}"), "not hl7")
                .await
                .is_empty()
        );
    }
}

fn mfn(w: &World, kind: &str, event: &str, row: &str) -> String {
    format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||MFN^{kind}|{}|P|2.5\rMFI|{}|||UPD|AL\rMFE|{event}||20261116|{}\r{row}\r",
        w.sender,
        Uuid::new_v4().simple(),
        if kind == "M02" { "STF" } else { "LOC" },
        Uuid::new_v4().simple()
    )
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn master_file_rows_add_change_and_retire_staff_and_beds() {
    let w = world().await;
    let staff_id = format!("S-{}", Uuid::new_v4().simple());
    let stf = |active: &str| format!("STF|{staff_id}||Okafor^Ada^M||F|19800105|{active}");
    for (event, active) in [("MAD", "A"), ("MUP", "A"), ("MUP", "I"), ("MDL", "I")] {
        let ack = post(
            &w,
            "/api/hl7/v2/mfn-staff",
            &mfn(&w, "M02", event, &stf(active)),
        )
        .await;
        assert!(
            ["AA", "AE"].contains(&code(&ack).as_str()),
            "{event}: {ack:?}"
        );
    }
    let bad = post(
        &w,
        "/api/hl7/v2/mfn-staff",
        &mfn(&w, "M02", "XXX", &stf("A")),
    )
    .await;
    assert_eq!(code(&bad), "AE", "{bad:?}");

    let bed = format!("MB-{}", Uuid::new_v4().simple());
    let loc = format!("LOC|WARD1^R1^{bed}|New bed");
    for event in ["MAD", "MUP", "MDL"] {
        let ack = post(&w, "/api/hl7/v2/mfn-location", &mfn(&w, "M05", event, &loc)).await;
        assert!(
            ["AA", "AE"].contains(&code(&ack).as_str()),
            "{event}: {ack:?}"
        );
    }
    let no_bed = post(
        &w,
        "/api/hl7/v2/mfn-location",
        &mfn(&w, "M05", "MAD", "LOC|WARD1^R1"),
    )
    .await;
    assert_eq!(code(&no_bed), "AE", "{no_bed:?}");
}

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
use patient_administration_system::upstream::{StubPersonClient, StubWorkerClient};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct World {
    s: AppState,
    sender: String,
    mrn: String,
    room_code: String,
    bed_code: String,
    workers: Arc<StubWorkerClient>,
}

async fn world() -> World {
    let workers = Arc::new(StubWorkerClient::new());
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()))
        .with_worker_client(workers.clone());
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
        room_code: format!("R{tag}"),
        bed_code,
        workers,
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

fn mfn(w: &World, file: &str, event: &str, key: &str, row: &str) -> String {
    format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||MFN^M02|{}|P|2.5\rMFI|{file}|||UPD|AL\rMFE|{event}||20261116|{key}\r{row}\r",
        w.sender,
        Uuid::new_v4().simple(),
    )
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn master_file_staff_rows_add_change_and_retire_workers() {
    let w = world().await;
    let id = format!("S-{}", Uuid::new_v4().simple());
    let stf = |active: &str| format!("STF|{id}||Okafor^Ada^M||F|19800105|{active}");
    let send = |event: &'static str, active: &'static str| {
        let (w, msg) = (&w, mfn(&w, "PRA", event, &id, &stf(active)));
        async move { post(w, "/api/hl7/v2/mfn-staff", &msg).await }
    };
    let mad = send("MAD", "A").await;
    assert_eq!(code(&mad), "AA", "{mad:?}");
    assert_eq!(w.workers.len(), 1);
    let again = send("MAD", "A").await;
    assert_eq!(code(&again), "AE", "already exists: {again:?}");
    let mup = send("MUP", "I").await;
    assert_eq!(code(&mup), "AA", "{mup:?}");
    let mdl = send("MDL", "I").await;
    assert_eq!(code(&mdl), "AA", "{mdl:?}");
    assert!(w.workers.is_empty());
    for event in ["MUP", "MDL"] {
        let gone = send(event, "A").await;
        assert_eq!(code(&gone), "AE", "{event} of nobody: {gone:?}");
    }

    // What makes a message the wrong one.
    let other_file = post(
        &w,
        "/api/hl7/v2/mfn-staff",
        &mfn(&w, "LOC", "MAD", &id, &stf("A")),
    )
    .await;
    assert_eq!(code(&other_file), "AE", "{other_file:?}");
    let bad_event = send("XXX", "A").await;
    assert_eq!(code(&bad_event), "AE", "{bad_event:?}");
    let no_mfi = format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||MFN^M02|{}|P|2.5\rMFE|MAD||20261116|x\r{}\r",
        w.sender,
        Uuid::new_v4().simple(),
        stf("A")
    );
    assert_eq!(
        code(&post(&w, "/api/hl7/v2/mfn-staff", &no_mfi).await),
        "AE"
    );
    let no_rows = mfn(&w, "PRA", "MAD", &id, "NTE|1");
    assert_eq!(
        code(&post(&w, "/api/hl7/v2/mfn-staff", &no_rows).await),
        "AE"
    );
    let orphan = format!(
        "MSH|^~\\&|{}|WARDS|PAS|HOSP|20261116120000||MFN^M02|{}|P|2.5\rMFI|PRA|||UPD|AL\r{}\r",
        w.sender,
        Uuid::new_v4().simple(),
        stf("A")
    );
    assert_eq!(
        code(&post(&w, "/api/hl7/v2/mfn-staff", &orphan).await),
        "AE"
    );
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn master_file_location_rows_add_change_and_retire_beds() {
    let w = world().await;
    let new_bed = format!("MB-{}", Uuid::new_v4().simple());
    let loc = |bed: &str, room: &str| format!("LOC|WARD1^{room}^{bed}|Bed name");
    let send = |event: &'static str, row: String| {
        let (w, msg) = (&w, mfn(&w, "LOC", event, &new_bed, &row));
        async move { post(w, "/api/hl7/v2/mfn-location", &msg).await }
    };
    let mad = send("MAD", loc(&new_bed, &w.room_code)).await;
    assert_eq!(code(&mad), "AA", "{mad:?}");
    let again = send("MAD", loc(&new_bed, &w.room_code)).await;
    assert_eq!(code(&again), "AE", "already exists: {again:?}");
    let no_room = send("MAD", loc("MB-NEW-2", "")).await;
    assert_eq!(code(&no_room), "AE", "{no_room:?}");
    let bad_room = send("MAD", loc("MB-NEW-3", "NO-SUCH-ROOM")).await;
    assert_eq!(code(&bad_room), "AE", "{bad_room:?}");
    let mup = send("MUP", loc(&new_bed, &w.room_code)).await;
    assert_eq!(code(&mup), "AA", "{mup:?}");
    let mdl = send("MDL", loc(&w.bed_code, &w.room_code)).await;
    assert_eq!(code(&mdl), "AA", "{mdl:?}");
    for event in ["MUP", "MDL"] {
        let gone = send(event, loc("NO-SUCH-BED", &w.room_code)).await;
        assert_eq!(code(&gone), "AE", "{event} of no bed: {gone:?}");
    }
    let no_bed = send("MAD", "LOC|WARD1^R1".to_string()).await;
    assert_eq!(code(&no_bed), "AE", "{no_bed:?}");
    let wrong_file = post(
        &w,
        "/api/hl7/v2/mfn-location",
        &mfn(&w, "PRA", "MAD", "X", &loc("X", "R")),
    )
    .await;
    assert_eq!(code(&wrong_file), "AE", "{wrong_file:?}");
}

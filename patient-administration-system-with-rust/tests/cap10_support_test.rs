//! CAP-10: support flags that can only add help (NFR-11). Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! The schema guard is the important one: it reads the live schema and fails if any column anywhere in it
//! scores, ranks, penalises or suppresses contact. A later feature that genuinely needs such a column (the
//! no-show probability of CAP-12) must add it to `ALLOWED` here, with its reason, in the same change, so
//! the decision is visible in review.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::standby::{CandidateFilter, StandbyInput};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

/// Words that mean a column scores, ranks, penalises or suppresses contact.
const FORBIDDEN: [&str; 12] = [
    "score",
    "rank",
    "penal",
    "suppress",
    "deprioriti",
    "overbook",
    "withhold",
    "no_contact",
    "do_not_contact",
    "skip_contact",
    "contact_block",
    "demote",
];

/// `(table.column, reason)` pairs that were allowed on purpose. Empty: nothing needs one yet.
const ALLOWED: [(&str, &str); 0] = [];

fn violates(column: &str) -> bool {
    let c = column.to_ascii_lowercase();
    FORBIDDEN.iter().any(|w| c.contains(w))
}

fn sys() -> UserContext {
    UserContext::system()
}

#[test]
fn test_the_guard_recognises_what_it_is_meant_to_catch() {
    for bad in [
        "risk_score",
        "priority_penalty",
        "suppress_contact",
        "noshow_rank",
        "deprioritised",
        "overbook_weight",
        "withhold_slot",
    ] {
        assert!(violates(bad), "{bad} should be caught");
    }
    for fine in [
        "priority",
        "status",
        "service_type",
        "needs_transport",
        "reason_code",
        "kind",
        "created_by",
        "seconds_to_refill",
    ] {
        assert!(!violates(fine), "{fine} should be allowed");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_column_in_the_schema_scores_ranks_penalises_or_suppresses() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let rows = s
        .db
        .query_all(Statement::from_string(
            DbBackend::Postgres,
            "SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public'",
        ))
        .await
        .unwrap();
    assert!(rows.len() > 100, "the guard must actually see the schema");
    let bad: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{}.{}",
                r.try_get::<String>("", "table_name").unwrap(),
                r.try_get::<String>("", "column_name").unwrap()
            )
        })
        .filter(|tc| violates(tc.split('.').nth(1).unwrap()))
        .filter(|tc| !ALLOWED.iter().any(|(a, _)| a == tc))
        .collect();
    assert!(
        bad.is_empty(),
        "columns that score, rank, penalise or suppress contact (NFR-11): {bad:?}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn rebooking_options_and_standby_candidates_are_the_same_with_and_without_flags() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;

    // Rebooking: one patient, a few free slots.
    let person = Uuid::new_v4();
    let own = common::free_slot(&s.db, schedule, 5).await;
    let appt = s
        .scheduling
        .book_slot(own.id, person, &sys())
        .await
        .unwrap();
    for d in [6, 7, 8, 9] {
        common::free_slot(&s.db, schedule, d).await;
    }
    let before: Vec<Uuid> = s
        .rebooking
        .offer(appt.id)
        .await
        .unwrap()
        .options
        .iter()
        .map(|o| o.slot_id)
        .collect();
    assert!(!before.is_empty());
    s.support
        .add(
            person,
            "needs_support_call",
            Some(appt.id),
            Some("anxiety"),
            "Nurse B",
            &sys(),
        )
        .await
        .unwrap();
    s.support
        .add(person, "navigator_referral", None, None, "Nurse B", &sys())
        .await
        .unwrap();
    // A fresh offer, so it is computed again rather than reused.
    s.db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "DELETE FROM rebook_offers WHERE appointment_id = $1",
        [appt.id.into()],
    ))
    .await
    .unwrap();
    let after: Vec<Uuid> = s
        .rebooking
        .offer(appt.id)
        .await
        .unwrap()
        .options
        .iter()
        .map(|o| o.slot_id)
        .collect();
    assert_eq!(before, after, "flags do not change what is offered");

    // Standby: the same people in the same order.
    let canceller = Uuid::new_v4();
    let slot = common::free_slot(&s.db, schedule, 12).await;
    let booked = s
        .scheduling
        .book_slot(slot.id, canceller, &sys())
        .await
        .unwrap();
    let people: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    for p in &people {
        s.standby
            .add(
                StandbyInput {
                    person_id: *p,
                    service_type: service.clone(),
                    short_notice_ok: true,
                    language: None,
                    needs_transport: false,
                    phone_only: false,
                },
                &sys(),
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    s.scheduling
        .cancel(
            booked.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    let rel =
        s.db.query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM slot_releases WHERE slot_id = $1 ORDER BY released_at DESC LIMIT 1",
            [slot.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let rid: Uuid = rel.try_get("", "id").unwrap();
    let order = |v: Vec<patient_administration_system::standby::StandbyEntry>| {
        v.into_iter().map(|e| e.person_id).collect::<Vec<_>>()
    };
    let plain = order(
        s.standby
            .candidates(rid, &CandidateFilter::default())
            .await
            .unwrap(),
    );
    assert_eq!(plain, people);
    // Flag the middle one, and the last.
    s.support
        .add(
            people[1],
            "needs_support_call",
            None,
            None,
            "Nurse B",
            &sys(),
        )
        .await
        .unwrap();
    s.support
        .add(
            people[2],
            "navigator_referral",
            None,
            Some("cannot_travel"),
            "Nurse B",
            &sys(),
        )
        .await
        .unwrap();
    let flagged = order(
        s.standby
            .candidates(rid, &CandidateFilter::default())
            .await
            .unwrap(),
    );
    assert_eq!(plain, flagged, "flags do not reorder, add or remove anyone");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_flag_needs_a_known_kind_a_name_and_a_person_to_resolve_it() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let p = Uuid::new_v4();
    assert!(
        s.support
            .add(p, "risk_flag", None, None, "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.support
            .add(p, "needs_support_call", None, None, " ", &sys())
            .await
            .is_err()
    );
    assert!(
        s.support
            .add(
                p,
                "needs_support_call",
                None,
                Some("Free text, not a code"),
                "A",
                &sys()
            )
            .await
            .is_err()
    );
    let id = s
        .support
        .add(
            p,
            "navigator_referral",
            None,
            Some("cost"),
            "Nurse B",
            &sys(),
        )
        .await
        .unwrap();
    assert!(
        s.support
            .open(500)
            .await
            .unwrap()
            .iter()
            .any(|f| f.id == id)
    );
    assert!(
        s.support
            .resolve(id, "banished", "Nurse C", &sys())
            .await
            .is_err()
    );
    assert!(s.support.resolve(id, "helped", "", &sys()).await.is_err());
    s.support
        .resolve(id, "helped", "Nurse C", &sys())
        .await
        .unwrap();
    assert!(
        s.support
            .open(500)
            .await
            .unwrap()
            .iter()
            .all(|f| f.id != id)
    );
    assert!(
        s.support
            .resolve(id, "helped", "Nurse C", &sys())
            .await
            .is_err(),
        "once"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_raise_list_and_resolve() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let p = Uuid::new_v4();
    let (st, _) = call(
        "POST",
        "/api/support-flags".into(),
        Some(serde_json::json!({ "person_id": p, "kind": "risk", "created_by": "A" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        "POST",
        "/api/support-flags".into(),
        Some(serde_json::json!({ "person_id": p, "kind": "needs_support_call", "reason_code": "anxiety", "created_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["data"]["id"].as_str().unwrap().to_string();
    let (st, v) = call("GET", "/api/support-flags/open?limit=500".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == id.as_str())
    );
    let (st, _) = call(
        "POST",
        format!("/api/support-flags/{id}/resolve"),
        Some(serde_json::json!({ "outcome": "helped", "resolved_by": "Nurse C" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "POST",
        format!("/api/support-flags/{id}/resolve"),
        Some(serde_json::json!({ "outcome": "helped", "resolved_by": "Nurse C" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

//! WPAS-3 (FR-217): patient enquiry shows what the PAS holds for a person, and one enquiry is one audited
//! read. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Duration;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::referrals::NewReferral;
use patient_administration_system::suspicions::{NewSuspicion, today};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn one_enquiry_shows_the_patient_and_writes_one_audit_row() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let ctx = UserContext::system();
    let person = Uuid::new_v4();
    let service = format!("wpas3-{}", Uuid::new_v4());

    // A referral with a suspicion, booked into an appointment.
    let (r, _) = s
        .referrals
        .create(
            NewReferral {
                person_id: person,
                source: "gp".into(),
                external_ref: None,
                target_service: service.clone(),
                reason: None,
                referring_worker_id: None,
                received_at: None,
            },
            &ctx,
        )
        .await
        .unwrap();
    s.suspicions
        .record(
            r.id,
            NewSuspicion {
                condition: "cancer".into(),
                subtype: None,
                subtype_scheme: None,
                unique_pathway_identifier: None,
                suspected_on: today() - Duration::days(6),
                suspected_by: "Dr Rhian Evans".into(),
                recorded_by: "Gareth Jones".into(),
            },
            &ctx,
        )
        .await
        .unwrap();
    let sched = common::new_schedule_for(&s.db, &service).await;
    common::free_slot_in_hours(&s.db, sched.id, 50).await;
    let booked = s.referrals.book_first_available(r.id, &ctx).await.unwrap();

    let reads = || async {
        let n: i64 =
            s.db.one(Stmt::new(
                "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'read'",
                [person.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col("n")
            .unwrap();
        n
    };
    let before = reads().await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{person}/enquiry"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    let d = &v["data"];
    assert_eq!(d["person_id"], person.to_string());
    assert_eq!(d["referrals"][0]["id"], r.id.to_string());
    assert_eq!(d["suspected_conditions"][0]["condition"], "cancer");
    assert_eq!(d["suspected_conditions"][0]["days_waited"], 6);
    assert_eq!(
        d["appointments"][0]["id"],
        booked.appointment.id.to_string()
    );
    for list in ["waitlist_entries", "rtt_pathways", "admissions", "letters"] {
        assert!(d[list].is_array(), "{list} is always a list");
    }
    assert_eq!(reads().await - before, 1, "one enquiry, one audited read");

    // Someone the PAS knows nothing of: empty lists, not an error.
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{}/enquiry", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

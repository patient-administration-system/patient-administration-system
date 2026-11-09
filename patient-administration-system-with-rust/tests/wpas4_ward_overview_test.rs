//! WPAS-4 (FR-218): the ward overview lists each bed and who is in it. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::Address;
use patient_administration_system::models::facility::{Bed, BedStatus, Facility, Room, Ward};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("if-match", "*")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let st = r.status();
    (
        st,
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
            .unwrap_or_default(),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_overview_lists_each_bed_and_who_is_in_it() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let u = Uuid::new_v4().simple().to_string();
    let f = Facility::new("F".into(), format!("F{u}"), Address::default());
    call(&app, "POST", "/api/facilities", serde_json::json!(f)).await;
    let w = Ward::new(f.id, "W".into(), format!("W{u}"), 3);
    call(&app, "POST", "/api/wards", serde_json::json!(w)).await;
    let r = Room::new(w.id, "R".into(), format!("R{u}"));
    call(&app, "POST", "/api/rooms", serde_json::json!(r)).await;
    let mut beds = Vec::new();
    for i in 0..3 {
        let b = Bed::new(r.id, format!("B{i}"), format!("B{i}{u}"));
        call(&app, "POST", "/api/beds", serde_json::json!(b)).await;
        beds.push(b.id);
    }
    let person = Uuid::new_v4();
    let admitted = s
        .adt
        .admit(person, beds[1], &UserContext::system())
        .await
        .unwrap();

    let o = s.resources.ward_overview(w.id).await.unwrap();
    assert_eq!(o.beds.len(), 3, "every bed of the ward");
    assert_eq!(o.occupancy.total_beds, 3);
    assert_eq!(
        o.beds.iter().map(|b| b.bed.as_str()).collect::<Vec<_>>(),
        ["B0", "B1", "B2"],
        "by room and bed name"
    );
    let taken = &o.beds[1];
    assert_eq!(taken.status, BedStatus::Occupied);
    assert_eq!(taken.person_id, Some(person));
    assert_eq!(taken.admission_id, Some(admitted.admission.id));
    assert_eq!(taken.encounter_id, Some(admitted.encounter.id));
    assert_eq!(taken.days_in_hospital, Some(0));
    assert!(taken.in_bed_since.is_some());
    for free in [&o.beds[0], &o.beds[2]] {
        assert_eq!(free.person_id, None);
        assert_eq!(free.days_in_hospital, None);
    }

    // Through the route, and nothing clinical in it: no reason, no diagnosis.
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/wards/{}/overview", w.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let text = v["data"].to_string();
    for clinical in ["reason", "diagnosis", "condition"] {
        assert!(
            !text.contains(clinical),
            "the overview shows nothing clinical ({clinical})"
        );
    }
}

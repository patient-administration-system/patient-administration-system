//! TOL-1: tolerances and red/amber/green tiers. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! The dev database holds whatever earlier runs left, so the status tests choose limits whose tier does not
//! depend on the data: a target no share can reach with no slack is red whenever there is anything to
//! measure, and a target of zero with a wide tolerance is green whenever there is.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::tolerance::{Limits, METRICS, Rag};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

fn limits(target: f64, tolerance: f64, escalate_beyond: f64) -> Limits {
    Limits {
        target,
        tolerance,
        escalate_beyond,
    }
}

async fn reset_all(s: &AppState) {
    for m in METRICS {
        s.tolerance.reset(m.key, &sys()).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_measure_starts_at_its_default_and_a_named_person_can_change_it() {
    let s = state().await;
    reset_all(&s).await;
    let before = s.tolerance.settings().await.unwrap();
    assert_eq!(before.len(), METRICS.len());
    assert!(before.iter().all(|x| !x.customised && x.set_by.is_none()));

    // A name is required, the tiers must be ordered, and the measure must exist.
    let m = "bed_occupancy";
    assert!(
        s.tolerance
            .set(m, limits(0.8, 0.05, 0.1), "  ", "", &sys())
            .await
            .is_err()
    );
    assert!(
        s.tolerance
            .set(m, limits(0.8, 0.2, 0.1), "Ops lead", "", &sys())
            .await
            .is_err(),
        "the limit is below the tolerance"
    );
    assert!(
        s.tolerance
            .set(m, limits(0.8, -0.1, 0.1), "Ops lead", "", &sys())
            .await
            .is_err()
    );
    assert!(
        s.tolerance
            .set(
                "not_a_measure",
                limits(1.0, 0.1, 0.2),
                "Ops lead",
                "",
                &sys()
            )
            .await
            .is_err()
    );

    s.tolerance
        .set(
            m,
            limits(0.9, 0.03, 0.08),
            "Ops lead",
            "winter plan",
            &sys(),
        )
        .await
        .unwrap();
    let now = s.tolerance.settings().await.unwrap();
    let x = now.iter().find(|x| x.metric == m).unwrap();
    assert!(x.customised);
    assert_eq!(x.limits, limits(0.9, 0.03, 0.08));
    assert_eq!(x.set_by.as_deref(), Some("Ops lead"));
    assert_eq!(x.note.as_deref(), Some("winter plan"));
    // Only that one changed.
    assert_eq!(now.iter().filter(|x| x.customised).count(), 1);

    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, m.as_bytes());
    assert!(common::audit_count(&s.db, id, "set").await >= 1);

    s.tolerance.reset(m, &sys()).await.unwrap();
    assert!(
        s.tolerance
            .settings()
            .await
            .unwrap()
            .iter()
            .all(|x| !x.customised)
    );
    assert!(common::audit_count(&s.db, id, "reset").await >= 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn status_puts_each_measure_in_its_tier_and_the_worst_one_wins() {
    let s = state().await;
    reset_all(&s).await;

    // Slot use: lower is worse. Target 5 (unreachable for a share) with no slack is red when there are slots.
    s.tolerance
        .set(
            "slot_utilisation",
            limits(5.0, 0.0, 0.0),
            "Ops lead",
            "",
            &sys(),
        )
        .await
        .unwrap();
    // Beds: either way is worse. Target 0 with a tolerance of 1 is green whenever there are beds.
    s.tolerance
        .set(
            "bed_occupancy",
            limits(0.0, 1.0, 1.0),
            "Ops lead",
            "",
            &sys(),
        )
        .await
        .unwrap();

    let st = s.tolerance.status(30).await.unwrap();
    assert_eq!(st.readings.len(), METRICS.len(), "every measure is read");
    let by = |k: &str| st.readings.iter().find(|r| r.metric == k).unwrap();

    let slots = by("slot_utilisation");
    assert!(slots.customised);
    if slots.sample > 0 {
        assert_eq!(slots.rag, Rag::Red);
        assert!(slots.value.is_some() && slots.adverse_drift.unwrap() >= 4.0);
    } else {
        assert_eq!(slots.rag, Rag::NoData, "nothing to measure is not green");
        assert!(slots.value.is_none());
    }
    let beds = by("bed_occupancy");
    if beds.sample > 0 {
        assert_eq!(beds.rag, Rag::Green);
    } else {
        assert_eq!(beds.rag, Rag::NoData);
    }

    // Overall is the worst tier among the readings that have data.
    let rank = |r: Rag| match r {
        Rag::NoData => 0,
        Rag::Green => 1,
        Rag::Amber => 2,
        Rag::Red => 3,
    };
    let worst = st.readings.iter().map(|r| rank(r.rag)).max().unwrap();
    assert_eq!(rank(st.overall), worst);
    // A reading with no value is never anything but no data, and one with a value never is.
    for r in &st.readings {
        assert_eq!(r.value.is_none(), r.rag == Rag::NoData, "{}", r.metric);
    }
    reset_all(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_list_set_read_and_reset() {
    let s = state().await;
    reset_all(&s).await;
    let app = patient_administration_system::api::rest::router(s.clone());
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

    let (st, v) = call("GET", "/api/tolerances".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let list = v["data"].as_array().unwrap();
    assert_eq!(list.len(), METRICS.len());
    assert!(list.iter().all(|x| x["customised"] == false));
    assert!(
        list.iter()
            .any(|x| x["metric"] == "wait_p90_weeks" && x["unit"] == "weeks")
    );

    let body = serde_json::json!({
        "target": 12.0, "tolerance": 1.0, "escalate_beyond": 4.0,
        "set_by": "Ops lead", "note": "shorter target"
    });
    let (st, _) = call("PUT", "/api/tolerances/wait_p90_weeks".into(), Some(body)).await;
    assert_eq!(st, StatusCode::OK);

    // Bad input is a 4xx, never a 5xx.
    let bad =
        serde_json::json!({"target": 1.0, "tolerance": 3.0, "escalate_beyond": 2.0, "set_by": "x"});
    let (st, _) = call("PUT", "/api/tolerances/wait_p90_weeks".into(), Some(bad)).await;
    assert!(st.is_client_error(), "{st}");
    let (st, _) = call(
        "PUT",
        "/api/tolerances/nope".into(),
        Some(serde_json::json!({"target": 1.0, "tolerance": 0.1, "escalate_beyond": 0.2, "set_by": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = call("GET", "/api/tolerances/status?days=30".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let wait = v["data"]["readings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["metric"] == "wait_p90_weeks")
        .unwrap()
        .clone();
    assert_eq!(wait["target"], 12.0);
    assert_eq!(wait["customised"], true);
    assert!(["green", "amber", "red", "no_data"].contains(&wait["rag"].as_str().unwrap()));
    assert!(["green", "amber", "red", "no_data"].contains(&v["data"]["overall"].as_str().unwrap()));

    let (st, _) = call("DELETE", "/api/tolerances/wait_p90_weeks".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call("GET", "/api/tolerances".into(), None).await;
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["customised"] == false)
    );
}

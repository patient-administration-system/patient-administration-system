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
use patient_administration_system::tolerance::{Limits, METRICS, Rag, Scope, ScopeKind};
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

/// Back to the built-in defaults everywhere, including any service or ward a person set.
async fn reset_all(s: &AppState) {
    for x in s.tolerance.settings().await.unwrap() {
        let scope = Scope {
            kind: x.scope_kind,
            key: x.scope_key.clone(),
        };
        if x.customised {
            s.tolerance.reset(x.metric, &scope, &sys()).await.unwrap();
        }
    }
}

/// How many measures there are for the whole service (the list also holds any per-service or per-ward rows).
fn whole_service(list: &[patient_administration_system::tolerance::Setting]) -> usize {
    list.iter()
        .filter(|x| x.scope_kind == ScopeKind::All)
        .count()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_measure_starts_at_its_default_and_a_named_person_can_change_it() {
    let s = state().await;
    reset_all(&s).await;
    let before = s.tolerance.settings().await.unwrap();
    assert_eq!(whole_service(&before), METRICS.len());
    assert_eq!(before.len(), METRICS.len(), "no scoped rows after a reset");
    assert!(before.iter().all(|x| !x.customised && x.set_by.is_none()));

    // A name is required, the tiers must be ordered, and the measure must exist.
    let m = "bed_occupancy";
    assert!(
        s.tolerance
            .set(m, &Scope::all(), limits(0.8, 0.05, 0.1), "  ", "", &sys())
            .await
            .is_err()
    );
    assert!(
        s.tolerance
            .set(
                m,
                &Scope::all(),
                limits(0.8, 0.2, 0.1),
                "Ops lead",
                "",
                &sys()
            )
            .await
            .is_err(),
        "the limit is below the tolerance"
    );
    assert!(
        s.tolerance
            .set(
                m,
                &Scope::all(),
                limits(0.8, -0.1, 0.1),
                "Ops lead",
                "",
                &sys()
            )
            .await
            .is_err()
    );
    assert!(
        s.tolerance
            .set(
                "not_a_measure",
                &Scope::all(),
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
            &Scope::all(),
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

    s.tolerance.reset(m, &Scope::all(), &sys()).await.unwrap();
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
            &Scope::all(),
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
            &Scope::all(),
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

// ---- Per service, per ward, and alerts ----

use sea_orm::{ConnectionTrait, DbBackend, Statement};

fn reading<'a>(
    list: &'a [patient_administration_system::tolerance::Reading],
    metric: &str,
    key: &str,
) -> &'a patient_administration_system::tolerance::Reading {
    list.iter()
        .find(|r| r.metric == metric && r.scope_key == key)
        .unwrap_or_else(|| panic!("no {metric} reading for {key}"))
}

/// A service with two past slots and nothing booked: its slot use is 0.
async fn idle_service(s: &AppState) -> String {
    let svc = format!("tol-svc-{}", Uuid::new_v4().simple());
    let sched = common::new_schedule_for(&s.db, &svc).await;
    common::free_slot(&s.db, sched.id, -1).await;
    common::free_slot(&s.db, sched.id, -2).await;
    svc
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_service_can_have_its_own_tolerance_and_otherwise_follows_the_whole_service() {
    let s = state().await;
    reset_all(&s).await;
    let svc = idle_service(&s).await;
    let mine = Scope::from_parts(Some(&svc), None).unwrap();

    // On the defaults an idle service is red for slot use, and its own row is listed.
    let st = s.tolerance.status(30).await.unwrap();
    let r = reading(&st.by_service, "slot_utilisation", &svc);
    assert_eq!(r.value, Some(0.0));
    assert_eq!(r.rag, Rag::Red);
    assert!(!r.customised);
    assert_eq!(r.sample, 2);

    // A tolerance for the whole service reaches the service when it has none of its own.
    s.tolerance
        .set(
            "slot_utilisation",
            &Scope::all(),
            limits(0.0, 1.0, 1.0),
            "Ops lead",
            "",
            &sys(),
        )
        .await
        .unwrap();
    let st = s.tolerance.status(30).await.unwrap();
    let r = reading(&st.by_service, "slot_utilisation", &svc);
    assert_eq!(r.rag, Rag::Green);
    assert!(r.customised, "a person set the limits being used");

    // The service's own row wins over the whole service's, and only for that service.
    s.tolerance
        .set(
            "slot_utilisation",
            &mine,
            limits(0.9, 0.0, 0.05),
            "Clinic manager",
            "this clinic must be full",
            &sys(),
        )
        .await
        .unwrap();
    let st = s.tolerance.status(30).await.unwrap();
    assert_eq!(
        reading(&st.by_service, "slot_utilisation", &svc).rag,
        Rag::Red
    );
    assert_eq!(
        reading(&st.by_service, "slot_utilisation", &svc).limits,
        limits(0.9, 0.0, 0.05)
    );
    let list = s.tolerance.settings().await.unwrap();
    assert!(
        list.iter()
            .any(|x| x.scope_key == svc && x.scope_kind == ScopeKind::Service && x.customised)
    );

    // Resetting the service falls back to the whole service's limits, not the default.
    s.tolerance
        .reset("slot_utilisation", &mine, &sys())
        .await
        .unwrap();
    let st = s.tolerance.status(30).await.unwrap();
    assert_eq!(
        reading(&st.by_service, "slot_utilisation", &svc).rag,
        Rag::Green
    );
    reset_all(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_tolerance_only_narrows_to_what_the_measure_is_read_by() {
    let s = state().await;
    reset_all(&s).await;
    let svc = Scope::from_parts(Some("cardiology"), None).unwrap();
    let ward = Scope::from_parts(None, Some("W1")).unwrap();
    let ok = limits(0.8, 0.05, 0.1);
    let set = |m: &'static str, scope: Scope| {
        let s = s.clone();
        async move { s.tolerance.set(m, &scope, ok, "Ops lead", "", &sys()).await }
    };
    assert!(
        set("slot_utilisation", ward.clone()).await.is_err(),
        "slot use is read per service"
    );
    assert!(
        set("bed_occupancy", svc.clone()).await.is_err(),
        "beds are read per ward"
    );
    assert!(
        set("invoice_collection_rate", svc.clone()).await.is_err(),
        "money is read for the whole service only"
    );
    assert!(Scope::from_parts(Some("cardiology"), Some("W1")).is_err());
    assert!(
        set(
            "slot_utilisation",
            Scope::from_parts(Some("  "), None).unwrap()
        )
        .await
        .is_ok(),
        "a blank service means the whole service"
    );
    reset_all(&s).await;
    assert!(set("bed_occupancy", ward).await.is_ok());
    reset_all(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_ward_is_judged_on_its_own_beds() {
    let s = state().await;
    reset_all(&s).await;
    let code = format!("TW{}", &Uuid::new_v4().simple().to_string()[..10]);
    let (fac, ward, room) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let run = |sql: String, vals: Vec<sea_orm::Value>| {
        let db = s.db.clone();
        async move {
            db.execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                vals,
            ))
            .await
            .unwrap();
        }
    };
    run(
        "INSERT INTO facilities (id, name, code, address, active, created_at, updated_at) VALUES ($1,$2,$3,'{}'::jsonb,true,now(),now())".into(),
        vec![fac.into(), code.clone().into(), format!("F{code}").into()],
    )
    .await;
    run(
        "INSERT INTO wards (id, facility_id, name, code, capacity, active, created_at, updated_at) VALUES ($1,$2,$3,$4,2,true,now(),now())".into(),
        vec![ward.into(), fac.into(), code.clone().into(), code.clone().into()],
    )
    .await;
    run(
        "INSERT INTO rooms (id, ward_id, name, code, active, created_at, updated_at) VALUES ($1,$2,$3,$4,true,now(),now())".into(),
        vec![room.into(), ward.into(), "r".into(), format!("R{code}").into()],
    )
    .await;
    for i in 0..2 {
        run(
            "INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1,$2,$3,$4,'occupied',now(),now())".into(),
            vec![Uuid::new_v4().into(), room.into(), format!("b{i}").into(), format!("B{i}{code}").into()],
        )
        .await;
    }

    // Both beds are full: 100% against a target of 85% (either way worse, 5 and 10 points) is red.
    let st = s.tolerance.status(30).await.unwrap();
    let r = reading(&st.by_ward, "bed_occupancy", &code);
    assert_eq!(r.value, Some(1.0));
    assert_eq!(r.sample, 2);
    assert_eq!(r.rag, Rag::Red);

    // This ward is meant to run full.
    s.tolerance
        .set(
            "bed_occupancy",
            &Scope::from_parts(None, Some(&code)).unwrap(),
            limits(1.0, 0.0, 0.1),
            "Ward sister",
            "",
            &sys(),
        )
        .await
        .unwrap();
    let st = s.tolerance.status(30).await.unwrap();
    assert_eq!(reading(&st.by_ward, "bed_occupancy", &code).rag, Rag::Green);
    reset_all(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_red_measure_opens_one_alert_that_closes_when_it_recovers() {
    let s = state().await;
    reset_all(&s).await;
    let svc = idle_service(&s).await;
    let mine = Scope::from_parts(Some(&svc), None).unwrap();
    let mine_alerts = |open: bool| {
        let s = s.clone();
        let svc = svc.clone();
        async move {
            s.tolerance
                .alerts(open)
                .await
                .unwrap()
                .into_iter()
                .filter(|a| a.scope_key == svc && a.metric == "slot_utilisation")
                .collect::<Vec<_>>()
        }
    };

    // Red: an alert opens, once, however many times it is checked.
    let first = s.tolerance.evaluate(30).await.unwrap();
    assert!(first.raised >= 1 && first.open >= 1);
    let again = s.tolerance.evaluate(30).await.unwrap();
    let open = mine_alerts(true).await;
    assert_eq!(open.len(), 1, "one alert, not one per check");
    assert!(again.raised == 0 || again.raised < first.raised);
    assert_eq!(open[0].value, 0.0);
    assert_eq!(open[0].escalate_beyond, 0.15);
    assert!(open[0].acknowledged_by.is_none());

    // The event was published once.
    let n: i64 = s
        .db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*)::bigint AS n FROM outbox_events WHERE event_type = 'ToleranceAlertRaised' AND payload->>'scope_key' = $1",
            [svc.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert_eq!(n, 1);

    // A named person acknowledges it, once; it stays open.
    let id = open[0].id;
    assert!(s.tolerance.acknowledge(id, "  ", "", &sys()).await.is_err());
    s.tolerance
        .acknowledge(id, "Duty manager", "looking at the rota", &sys())
        .await
        .unwrap();
    assert!(
        s.tolerance
            .acknowledge(id, "Someone else", "", &sys())
            .await
            .is_err(),
        "already acknowledged"
    );
    assert!(
        s.tolerance
            .acknowledge(Uuid::new_v4(), "Duty manager", "", &sys())
            .await
            .is_err()
    );
    let open = mine_alerts(true).await;
    assert_eq!(open[0].acknowledged_by.as_deref(), Some("Duty manager"));
    assert_eq!(common::audit_count(&s.db, id, "acknowledge").await, 1);

    // The measure recovers (this clinic's tolerance is widened): the next check closes the alert.
    s.tolerance
        .set(
            "slot_utilisation",
            &mine,
            limits(0.0, 1.0, 1.0),
            "Clinic manager",
            "",
            &sys(),
        )
        .await
        .unwrap();
    let done = s.tolerance.evaluate(30).await.unwrap();
    assert!(done.cleared >= 1);
    assert!(mine_alerts(true).await.is_empty());
    let all = mine_alerts(false).await;
    assert_eq!(all.len(), 1);
    assert!(all[0].cleared_at.is_some());

    // Red again later: a new alert, because the old one is closed.
    s.tolerance
        .reset("slot_utilisation", &mine, &sys())
        .await
        .unwrap();
    s.tolerance.evaluate(30).await.unwrap();
    assert_eq!(mine_alerts(false).await.len(), 2);
    assert_eq!(mine_alerts(true).await.len(), 1);
    reset_all(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_take_a_service_and_serve_the_alerts() {
    let s = state().await;
    reset_all(&s).await;
    let svc = idle_service(&s).await;
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

    let body = serde_json::json!({
        "target": 0.9, "tolerance": 0.0, "escalate_beyond": 0.05,
        "set_by": "Clinic manager", "service": svc
    });
    let (st, _) = call("PUT", "/api/tolerances/slot_utilisation".into(), Some(body)).await;
    assert_eq!(st, StatusCode::OK);
    let both = serde_json::json!({
        "target": 0.9, "tolerance": 0.0, "escalate_beyond": 0.05,
        "set_by": "x", "service": svc, "ward": "W1"
    });
    let (st, _) = call("PUT", "/api/tolerances/slot_utilisation".into(), Some(both)).await;
    assert!(st.is_client_error(), "{st}");

    let (_, v) = call("GET", "/api/tolerances".into(), None).await;
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["scope_kind"] == "service" && x["scope_key"] == svc.as_str())
    );

    let (st, v) = call("GET", "/api/tolerances/status?days=30".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let mine = v["data"]["by_service"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["metric"] == "slot_utilisation" && r["scope_key"] == svc.as_str())
        .unwrap()
        .clone();
    assert_eq!(mine["rag"], "red");
    assert_eq!(v["data"]["overall"], "red");

    let (st, v) = call("POST", "/api/tolerances/evaluate?days=30".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["data"]["open"].as_i64().unwrap() >= 1);
    let (st, v) = call("GET", "/api/tolerance-alerts?open=true".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let alert = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["scope_key"] == svc.as_str())
        .unwrap()
        .clone();
    let id = alert["id"].as_str().unwrap();
    let (st, _) = call(
        "POST",
        format!("/api/tolerance-alerts/{id}/acknowledge"),
        Some(serde_json::json!({"acknowledged_by": "Duty manager"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "POST",
        format!("/api/tolerance-alerts/{id}/acknowledge"),
        Some(serde_json::json!({"acknowledged_by": "Again"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    let (st, _) = call(
        "DELETE",
        format!("/api/tolerances/slot_utilisation?service={svc}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    reset_all(&s).await;
}

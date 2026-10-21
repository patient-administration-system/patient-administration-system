//! The measurement feed: the postcode lookup a health board loads, and the job that fills
//! `person_demographics` from it. Needs PostgreSQL: `cargo test -- --ignored --test-threads=1`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Datelike, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

fn person(id: Uuid, born: Option<&str>, postcode: Option<&str>) -> Person {
    let mut data = serde_json::Map::new();
    if let Some(b) = born {
        data.insert("birth_date".into(), b.into());
    }
    if let Some(p) = postcode {
        data.insert(
            "addresses".into(),
            serde_json::json!([{ "line": ["1 Test Street"], "postal_code": p }]),
        );
    }
    Person { id, data }
}

async fn with_appointment(s: &patient_administration_system::api::rest::AppState, p: Uuid) {
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 9).await;
    s.scheduling.book_slot(slot.id, p, &sys()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_lookup_loads_good_lines_counts_bad_ones_and_needs_a_named_dataset() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let tag = Uuid::new_v4().as_u128() % 90 + 10;
    let csv = format!(
        "postcode,quintile\nzz{tag} 1ab,2\nZZ{tag}2AB, 5\nbad,1\nZZ{tag}3AB,9\nZZ{tag}4AB,1,extra\n"
    );
    assert!(
        s.analytics
            .load_deprivation_lookup(&csv, " ", "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.analytics
            .load_deprivation_lookup(&csv, "WIMD test", " ", &sys())
            .await
            .is_err()
    );
    let (loaded, rejected) = s
        .analytics
        .load_deprivation_lookup(&csv, "WIMD test", "Analyst A", &sys())
        .await
        .unwrap();
    assert_eq!(
        (loaded, rejected),
        (2, 3),
        "the header is not a rejection; three lines are"
    );
    let row =
        s.db.query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("SELECT quintile FROM deprivation_lookup WHERE postcode = 'ZZ{tag}1AB'"),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<i16>("", "quintile").unwrap(),
        2,
        "stored upper-case with no space"
    );
    // Loading again replaces rather than duplicates.
    let again = format!("ZZ{tag}1AB,4\n");
    s.analytics
        .load_deprivation_lookup(&again, "WIMD test 2", "Analyst A", &sys())
        .await
        .unwrap();
    let row =
        s.db.query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!(
                "SELECT quintile, dataset FROM deprivation_lookup WHERE postcode = 'ZZ{tag}1AB'"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i16>("", "quintile").unwrap(), 4);
    assert_eq!(row.try_get::<String>("", "dataset").unwrap(), "WIMD test 2");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_job_fills_age_band_and_quintile_keeps_only_the_bands_and_does_it_once() {
    let stub = Arc::new(StubPersonClient::new());
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub.clone());
    let tag = Uuid::new_v4().as_u128() % 90 + 10;
    s.analytics
        .load_deprivation_lookup(&format!("ZZ{tag}1AB,2\n"), "WIMD test", "Analyst A", &sys())
        .await
        .unwrap();

    let year = Utc::now().year();
    let (both, age_only, none, unknown_pc) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    stub.insert(person(
        both,
        Some(&format!("{}-01-01", year - 30)),
        Some(&format!("zz{tag} 1ab")),
    ));
    stub.insert(person(
        age_only,
        Some(&format!("{}-01-01", year - 80)),
        None,
    ));
    stub.insert(person(none, None, None));
    stub.insert(person(
        unknown_pc,
        Some(&format!("{}-01-01", year - 20)),
        Some("ZZ99 9ZZ"),
    ));
    for p in [both, age_only, none, unknown_pc] {
        with_appointment(&s, p).await;
    }
    let upstream = patient_administration_system::upstream::UserContext::default();
    let n = s
        .analytics
        .refresh_demographics(
            s.person_client.as_ref().unwrap().as_ref(),
            5000,
            &upstream,
            &sys(),
        )
        .await
        .unwrap();
    assert!(n >= 3, "{n}");

    let get = |p: Uuid| {
        let db = s.db.clone();
        async move {
            db.query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT age_band, deprivation_quintile, source FROM person_demographics WHERE person_id = $1",
                [p.into()],
            ))
            .await
            .unwrap()
            .map(|r| {
                (
                    r.try_get::<Option<String>>("", "age_band").unwrap(),
                    r.try_get::<Option<i16>>("", "deprivation_quintile").unwrap(),
                    r.try_get::<String>("", "source").unwrap(),
                )
            })
        }
    };
    let b = get(both).await.unwrap();
    assert_eq!((b.0.as_deref(), b.1), (Some("25-34"), Some(2)));
    assert!(b.2.contains("WIMD test"), "{}", b.2);
    assert_eq!(get(age_only).await.unwrap().0.as_deref(), Some("75+"));
    assert_eq!(
        get(age_only).await.unwrap().1,
        None,
        "no postcode, no quintile"
    );
    assert_eq!(
        get(unknown_pc).await.unwrap().1,
        None,
        "a postcode not in the lookup is unknown, not guessed"
    );
    assert!(get(none).await.is_none(), "nothing known, nothing stored");

    // Done once: a second run does not touch them again.
    let before = get(both).await;
    s.analytics
        .refresh_demographics(
            s.person_client.as_ref().unwrap().as_ref(),
            5000,
            &upstream,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(get(both).await, before);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_http_endpoints_load_and_refresh() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let tag = Uuid::new_v4().as_u128() % 90 + 10;
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/analytics/deprivation-lookup?dataset=WIMD%20test&loaded_by=Analyst%20A")
                .body(Body::from(format!("ZZ{tag}1AB,3\nnonsense\n")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(
        (v["data"]["loaded"].as_u64(), v["data"]["rejected"].as_u64()),
        (Some(1), Some(1))
    );
    // No person-service configured: said plainly, not a crash.
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/analytics/demographics/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        r.status().is_server_error() || r.status() == StatusCode::BAD_GATEWAY,
        "{}",
        r.status()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_caller_has_a_daily_query_budget_so_differencing_across_queries_is_bounded_and_visible() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    // SAFETY: single-threaded (see the header).
    unsafe { std::env::set_var("PAS_ANALYTICS_QUERY_BUDGET", "3") };
    let me = |name: &str| UserContext {
        user_id: Some(format!("{name}-{}", Uuid::new_v4())),
        ..UserContext::system()
    };
    let (alice, bob) = (me("alice"), me("bob"));
    for i in 0..3 {
        s.analytics
            .charge_query(
                "appointment-outcomes",
                serde_json::json!({ "i": i }),
                &alice,
            )
            .await
            .unwrap();
    }
    let e = s
        .analytics
        .charge_query("appointment-outcomes", serde_json::json!({}), &alice)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("budget"), "{e}");
    // Another caller has their own budget, and the first caller's queries are on the record with their shape.
    s.analytics
        .charge_query("refill-rates", serde_json::json!({}), &bob)
        .await
        .unwrap();
    let row = s
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*)::bigint AS n FROM audit_log WHERE action = 'analytics_query' AND user_id = $1",
            [alice.user_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 3);
    unsafe { std::env::remove_var("PAS_ANALYTICS_QUERY_BUDGET") };
}

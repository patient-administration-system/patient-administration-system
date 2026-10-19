//! PILOT-10: refill rate of released slots by notice period, early-release share, deprivation quintile and
//! language, measured before any risk score. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test uses specialties of its own, so other data in the database cannot change its cells.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::analytics::{RefillCell, RefillDim, RefillReport};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

fn short() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

struct Released {
    person: Uuid,
    slot: Uuid,
}

/// `n` patients book a slot `days` ahead on `specialty` and cancel it, so each cancel is one release.
async fn releases(s: &AppState, specialty: &str, n: usize, days: i64) -> Vec<Released> {
    let schedule = common::new_schedule_for(&s.db, specialty).await.id;
    let mut out = Vec::new();
    for _ in 0..n {
        let slot = common::free_slot(&s.db, schedule, days).await;
        let person = Uuid::new_v4();
        let a = s
            .scheduling
            .book_slot(slot.id, person, &sys())
            .await
            .unwrap();
        s.scheduling
            .cancel(
                a.id,
                patient_administration_system::models::appointment::CancellationReason::PatientRequest,
                &sys(),
            )
            .await
            .unwrap();
        out.push(Released {
            person,
            slot: slot.id,
        });
    }
    out
}

/// Someone else books the released slot again.
async fn refill(s: &AppState, r: &Released) {
    s.scheduling
        .book_slot(r.slot, Uuid::new_v4(), &sys())
        .await
        .unwrap();
}

/// The slot's time has passed with nobody in it.
async fn let_expire(s: &AppState, r: &Released) {
    s.db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE slot_releases SET start_datetime = now() - interval '1 day' WHERE slot_id = $1 AND status = 'open'",
        [r.slot.into()],
    ))
    .await
    .unwrap();
}

async fn report(s: &AppState, dims: &[RefillDim]) -> RefillReport {
    s.analytics
        .refill_rates(
            dims,
            Some(Utc::now() - Duration::hours(1)),
            Some(Utc::now() + Duration::hours(1)),
            None,
        )
        .await
        .unwrap()
}

fn cell<'a>(r: &'a RefillReport, dim: &str, value: &str) -> &'a RefillCell {
    r.cells
        .iter()
        .find(|c| c.dims.iter().any(|(d, v)| d == dim && v == value))
        .unwrap_or_else(|| panic!("no cell {dim}={value}"))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_refill_rate_counts_only_releases_with_a_known_outcome() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let a = format!("a{}", short());
    let rs = releases(&s, &a, 16, 5).await;
    for r in &rs[..10] {
        refill(&s, r).await; // refilled
    }
    for r in &rs[10..12] {
        let_expire(&s, r).await; // the time passed unfilled
    }
    // The last four are still open, a week away: not a failure yet.
    let rep = report(&s, &[RefillDim::Specialty]).await;
    let c = cell(&rep, "specialty", &a);
    assert_eq!(
        c.resolved,
        Some(12),
        "10 refilled + 2 expired; the 4 still open are not counted"
    );
    assert_eq!(c.refilled, Some(10));
    assert_eq!(c.refill_rate, Some(0.833));
    assert_eq!(
        c.early_release_share,
        Some(1.0),
        "five days' notice is an early release"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn notice_period_and_early_release_share_are_measured_apart() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let (early, late) = (format!("e{}", short()), format!("l{}", short()));
    let es = releases(&s, &early, 12, 8).await;
    for r in &es {
        refill(&s, r).await;
    }
    // Cancelled a few hours before: a late release.
    let schedule = common::new_schedule_for(&s.db, &late).await.id;
    let mut ls = Vec::new();
    for _ in 0..12 {
        let slot = common::free_slot_in_hours(&s.db, schedule, 5).await;
        let a = s
            .scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .unwrap();
        s.scheduling
            .cancel(a.id, patient_administration_system::models::appointment::CancellationReason::PatientRequest, &sys())
            .await
            .unwrap();
        ls.push(Released {
            person: Uuid::new_v4(),
            slot: slot.id,
        });
    }
    for r in &ls[..3] {
        refill(&s, r).await;
    }
    for r in &ls[3..] {
        let_expire(&s, r).await;
    }
    let rep = report(&s, &[RefillDim::Specialty, RefillDim::NoticeBand]).await;
    let by = |sp: &str, band: &str| {
        rep.cells
            .iter()
            .find(|c| {
                c.dims
                    == vec![
                        ("specialty".into(), sp.into()),
                        ("notice_band".into(), band.into()),
                    ]
            })
            .unwrap_or_else(|| panic!("no cell {sp}/{band}"))
    };
    let e = by(&early, "7_plus_days");
    assert_eq!(
        (e.resolved, e.refill_rate, e.early_release_share),
        (Some(12), Some(1.0), Some(1.0))
    );
    let l = by(&late, "0_days");
    assert_eq!(
        (l.resolved, l.refill_rate, l.early_release_share),
        (Some(12), Some(0.25), Some(0.0))
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_rate_can_be_split_by_deprivation_quintile_and_language() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let a = format!("a{}", short());
    let rs = releases(&s, &a, 24, 5).await;
    for (i, r) in rs.iter().enumerate() {
        let (q, lang) = if i < 12 { (1, "en-001") } else { (5, "cy-001") };
        s.analytics
            .set_demographics(r.person, Some("45-54"), Some(q), "ETL", &sys())
            .await
            .unwrap();
        s.db.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO patient_communication_profiles (person_id, preferred_language) VALUES ($1,$2) \
             ON CONFLICT (person_id) DO UPDATE SET preferred_language = $2",
            [r.person.into(), lang.into()],
        ))
        .await
        .unwrap();
        // Everyone in quintile 1 is refilled; in quintile 5 only a quarter are.
        if i < 12 || i % 4 == 0 {
            refill(&s, r).await;
        } else {
            let_expire(&s, r).await;
        }
    }
    let rep = report(&s, &[RefillDim::Specialty, RefillDim::DeprivationQuintile]).await;
    let q = |n: &str| {
        rep.cells
            .iter()
            .find(|c| {
                c.dims
                    == vec![
                        ("specialty".into(), a.clone()),
                        ("deprivation_quintile".into(), n.into()),
                    ]
            })
            .unwrap()
    };
    assert_eq!(q("1").refill_rate, Some(1.0));
    assert_eq!(
        q("5").refill_rate,
        Some(0.25),
        "the gap this measurement exists to show"
    );

    let rep = report(&s, &[RefillDim::Specialty, RefillDim::Language]).await;
    let l = |n: &str| {
        rep.cells
            .iter()
            .find(|c| {
                c.dims
                    == vec![
                        ("specialty".into(), a.clone()),
                        ("language".into(), n.into()),
                    ]
            })
            .unwrap()
    };
    assert_eq!(l("en-001").refill_rate, Some(1.0));
    assert_eq!(l("cy-001").refill_rate, Some(0.25));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_small_cell_is_withheld_whole_and_no_identifier_appears() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let (big, small, other) = (
        format!("b{}", short()),
        format!("s{}", short()),
        format!("o{}", short()),
    );
    let rb = releases(&s, &big, 12, 5).await;
    for r in &rb {
        refill(&s, r).await;
    }
    let rs = releases(&s, &small, 3, 5).await;
    for r in &rs {
        refill(&s, r).await;
    }
    let ro = releases(&s, &other, 2, 5).await;
    for r in &ro {
        refill(&s, r).await;
    }
    let rep = report(&s, &[RefillDim::Specialty]).await;
    for name in [&small, &other] {
        let c = cell(&rep, "specialty", name);
        assert!(c.suppressed);
        assert_eq!(
            (c.resolved, c.refilled, c.refill_rate, c.early_release_share),
            (None, None, None, None)
        );
    }
    assert_eq!(cell(&rep, "specialty", &big).resolved, Some(12));
    assert!(
        rep.total_resolved >= 17,
        "the total counts the withheld cells too"
    );

    let json = serde_json::to_string(&rep).unwrap();
    let csv = rep.to_csv();
    for r in rb.iter().chain(&rs).chain(&ro) {
        for id in [r.person.to_string(), r.slot.to_string()] {
            assert!(
                !json.contains(&id) && !csv.contains(&id),
                "{id} must not appear"
            );
        }
    }
    assert!(csv.contains("suppressed"));
    let low = s
        .analytics
        .refill_rates(&[RefillDim::Specialty], None, None, Some(1))
        .await
        .unwrap();
    assert_eq!(low.k, 10, "k cannot be lowered");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_works_and_refuses_unknown_dimensions() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let a = format!("a{}", short());
    let rs = releases(&s, &a, 12, 5).await;
    for r in &rs {
        refill(&s, r).await;
    }
    // Two small cells of our own, so that suppression never has to hide a cell of ours as a second cell
    // whatever else is in the database.
    for n in [3, 2] {
        for r in releases(&s, &format!("t{}", short()), n, 5).await {
            refill(&s, &r).await;
        }
    }
    let app = patient_administration_system::api::rest::router(s);
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let st = r.status();
            (
                st,
                String::from_utf8(to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec())
                    .unwrap(),
            )
        }
    };
    let (st, body) = get("/api/analytics/refill-rates?group_by=specialty".into()).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body.contains(&a));
    // An explicit end a little ahead, so a database clock a moment ahead of this one cannot hide the releases.
    let to = (Utc::now() + Duration::hours(1))
        .to_rfc3339()
        .replace('+', "%2B");
    let (st, csv) = get(format!(
        "/api/analytics/refill-rates?group_by=specialty&format=csv&to={to}"
    ))
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(csv.starts_with("specialty,resolved,refilled,refill_rate,early_release_share\n"));
    assert!(csv.contains(&format!("{a},12,12,1.000,1.000")), "{csv}");
    for bad in [
        "group_by=person_id",
        "group_by=",
        "group_by=specialty,specialty",
    ] {
        let (st, _) = get(format!("/api/analytics/refill-rates?{bad}")).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
}

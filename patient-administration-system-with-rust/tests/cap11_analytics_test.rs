//! CAP-11: the privacy-safe analytics feed. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test builds its appointments inside a time window of its own, years ahead, so other data in the
//! database cannot change its counts.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use patient_administration_system::analytics::Dim;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::schedule::Slot;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    centre: DateTime<Utc>,
}

async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    // A day nobody has used. Appointments are never deleted, so on a development database the days of earlier
    // runs pile up, and a random day eventually lands on one of them and adds its appointments to this test's
    // counts. Look for an empty day instead of hoping.
    for _ in 0..200 {
        let d = 2000 + (Uuid::new_v4().as_u128() % 40_000) as i64;
        // Noon, so that the window (12 hours either side) is exactly one calendar day whatever the time now.
        let centre = (Utc::now() + Duration::days(d))
            .date_naive()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc();
        let used: i64 = s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM appointments WHERE start_datetime >= $1 AND start_datetime < $2",
                [(centre - Duration::hours(13)).into(), (centre + Duration::hours(13)).into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "n")
            .unwrap();
        if used == 0 {
            return W { s, centre };
        }
    }
    panic!("no unused day found in 200 tries");
}

impl W {
    fn window(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        (
            self.centre - Duration::hours(12),
            self.centre + Duration::hours(12),
        )
    }
    /// `n` booked appointments on a specialty of its own, at `hour` o'clock UTC on the window's day.
    async fn book(&self, specialty: &str, n: usize, hour: i64) -> Vec<(Uuid, Uuid)> {
        let schedule = common::new_schedule_for(&self.s.db, specialty).await.id;
        let day = self
            .centre
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let mut out = Vec::new();
        for i in 0..n {
            let start = day + Duration::hours(hour) + Duration::minutes(i as i64);
            let slot =
                patient_administration_system::db::repositories::slot::SlotRepository::create(
                    &self.s.db,
                    &Slot::new(schedule, start, start + Duration::minutes(15)),
                )
                .await
                .unwrap();
            let person = Uuid::new_v4();
            let a = self
                .s
                .scheduling
                .book_slot(slot.id, person, &sys())
                .await
                .unwrap();
            out.push((a.id, person));
        }
        out
    }
    async fn count_in_window(&self) -> i64 {
        let (from, to) = self.window();
        self.s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM appointments WHERE deleted_at IS NULL AND start_datetime >= $1 AND start_datetime < $2",
                [from.into(), to.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "n")
            .unwrap()
    }
}

fn short() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn cell<'a>(
    r: &'a patient_administration_system::analytics::Report,
    dim: &str,
    value: &str,
) -> &'a patient_administration_system::analytics::Cell {
    r.cells
        .iter()
        .find(|c| c.dims.iter().any(|(d, v)| d == dim && v == value))
        .unwrap_or_else(|| panic!("no cell {dim}={value}"))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cell_under_k_is_withheld_not_zero_and_the_totals_reconcile() {
    let w = world().await;
    let (a, b, c, d) = (
        format!("a{}", short()),
        format!("b{}", short()),
        format!("c{}", short()),
        format!("d{}", short()),
    );
    w.book(&a, 12, 9).await;
    w.book(&b, 3, 9).await;
    w.book(&c, 2, 9).await;
    w.book(&d, 15, 9).await;
    let (from, to) = w.window();
    let r =
        w.s.analytics
            .outcomes(&[Dim::Specialty], Some(from), Some(to), None)
            .await
            .unwrap();
    assert_eq!(r.k, 10);
    assert_eq!(cell(&r, "specialty", &a).count, Some(12));
    assert_eq!(cell(&r, "specialty", &d).count, Some(15));
    for small in [&b, &c] {
        let cl = cell(&r, "specialty", small);
        assert!(cl.suppressed);
        assert_eq!(cl.count, None, "withheld, not shown as zero");
    }
    assert_eq!(r.suppressed_cells, 2);
    // The grand total is the true count of every appointment in the range, shown or not.
    assert_eq!(r.total, w.count_in_window().await);
    assert_eq!(r.total, 12 + 3 + 2 + 15);
    let shown: i64 = r.cells.iter().filter_map(|c| c.count).sum();
    assert_eq!(shown, 27);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn one_hidden_cell_cannot_be_read_off_the_total() {
    let w = world().await;
    let (a, b, c) = (
        format!("a{}", short()),
        format!("b{}", short()),
        format!("c{}", short()),
    );
    w.book(&a, 12, 9).await;
    w.book(&b, 3, 9).await;
    w.book(&c, 15, 9).await;
    let (from, to) = w.window();
    let r =
        w.s.analytics
            .outcomes(&[Dim::Specialty], Some(from), Some(to), None)
            .await
            .unwrap();
    assert_eq!(
        r.suppressed_cells, 2,
        "the small cell and the smallest visible one"
    );
    assert!(cell(&r, "specialty", &b).suppressed);
    assert!(
        cell(&r, "specialty", &a).suppressed,
        "secondary suppression"
    );
    assert_eq!(cell(&r, "specialty", &c).count, Some(15));
    // total - visible = 15 hidden between two cells: it does not say which is 3.
    assert_eq!(r.total - 15, 15);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_caller_can_raise_k_but_never_lower_it() {
    let w = world().await;
    let a = format!("a{}", short());
    w.book(&a, 12, 9).await;
    let (from, to) = w.window();
    let low =
        w.s.analytics
            .outcomes(&[Dim::Specialty], Some(from), Some(to), Some(1))
            .await
            .unwrap();
    assert_eq!(low.k, 10, "asking for k=1 gets the configured minimum");
    let high =
        w.s.analytics
            .outcomes(&[Dim::Specialty], Some(from), Some(to), Some(20))
            .await
            .unwrap();
    assert_eq!(high.k, 20);
    assert!(cell(&high, "specialty", &a).suppressed, "12 is under 20");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_identifier_appears_in_the_json_or_the_csv() {
    let w = world().await;
    let a = format!("a{}", short());
    let booked = w.book(&a, 12, 9).await;
    let (from, to) = w.window();
    let r =
        w.s.analytics
            .outcomes(&[Dim::Specialty, Dim::Outcome], Some(from), Some(to), None)
            .await
            .unwrap();
    let json = serde_json::to_string(&r).unwrap();
    let csv = r.to_csv();
    for (appt, person) in booked {
        for id in [appt.to_string(), person.to_string()] {
            assert!(
                !json.contains(&id) && !csv.contains(&id),
                "{id} must not appear"
            );
        }
    }
    let keys = [
        "person_id",
        "appointment_id",
        "patient",
        "name",
        "mobile",
        "id\"",
    ];
    for k in keys {
        assert!(!json.contains(k), "the feed must not carry {k}: {json}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn age_band_deprivation_time_of_day_and_unknown_are_dimensions() {
    let w = world().await;
    let a = format!("a{}", short());
    let people = w.book(&a, 12, 9).await; // morning
    w.book(&a, 11, 14).await; // afternoon
    for (_, p) in &people {
        w.s.analytics
            .set_demographics(*p, Some("65-74"), Some(2), "ETL", &sys())
            .await
            .unwrap();
    }
    let (from, to) = w.window();
    let r =
        w.s.analytics
            .outcomes(
                &[Dim::AgeBand, Dim::DeprivationQuintile],
                Some(from),
                Some(to),
                None,
            )
            .await
            .unwrap();
    let banded = r
        .cells
        .iter()
        .find(|c| {
            c.dims
                == vec![
                    ("age_band".into(), "65-74".into()),
                    ("deprivation_quintile".into(), "2".into()),
                ]
        })
        .unwrap();
    assert_eq!(banded.count, Some(12));
    let unknown = r.cells.iter().find(|c| c.dims[0].1 == "unknown").unwrap();
    assert_eq!(
        unknown.count,
        Some(11),
        "people with no bands are counted as unknown, so totals reconcile: {:?}",
        r.cells
    );
    assert_eq!(r.total, 23);

    let t =
        w.s.analytics
            .outcomes(&[Dim::TimeOfDay], Some(from), Some(to), None)
            .await
            .unwrap();
    assert_eq!(t.total, 23);
    assert!(t.cells.iter().any(|c| c.dims[0].1 == "morning"
        || c.dims[0].1 == "afternoon"
        || c.dims[0].1 == "evening"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn outcome_and_notice_split_cancellations_misses_and_the_rest() {
    let w = world().await;
    let a = format!("a{}", short());
    let all = w.book(&a, 36, 9).await;
    for (i, (id, _)) in all.iter().enumerate() {
        match i % 3 {
            0 => {
                w.s.scheduling
                    .cancel(*id, patient_administration_system::models::appointment::CancellationReason::PatientRequest, &sys())
                    .await
                    .unwrap();
            }
            1 => {
                w.s.scheduling.mark_no_show(*id, &sys()).await.unwrap();
            }
            _ => {}
        }
    }
    let (from, to) = w.window();
    let r =
        w.s.analytics
            .outcomes(&[Dim::Outcome, Dim::Notice], Some(from), Some(to), None)
            .await
            .unwrap();
    let find = |o: &str, n: &str| {
        r.cells
            .iter()
            .find(|c| c.dims == vec![("outcome".into(), o.into()), ("notice".into(), n.into())])
            .and_then(|c| c.count)
    };
    assert_eq!(
        find("cancelled", "early_release"),
        Some(12),
        "days of notice"
    );
    assert_eq!(find("no_show", "silent_miss"), Some(12));
    assert_eq!(find("booked", "not_released"), Some(12));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_gives_json_and_csv_and_refuses_bad_input() {
    // These tests ask as an anonymous caller, and a development database keeps every earlier run's queries on
    // the record; the daily budget has its own test (`cap11_feed_test.rs`).
    // SAFETY: only ever raises a limit, so racing another test in this binary is harmless.
    unsafe { std::env::set_var("PAS_ANALYTICS_QUERY_BUDGET", "100000") };
    let w = world().await;
    let a = format!("a{}", short());
    w.book(&a, 12, 9).await;
    let (from, to) = w.window();
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let st = r.status();
            let ct = r
                .headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap().to_string())
                .unwrap_or_default();
            (
                st,
                ct,
                String::from_utf8(to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec())
                    .unwrap(),
            )
        }
    };
    let q = format!(
        "from={}&to={}",
        from.to_rfc3339().replace('+', "%2B"),
        to.to_rfc3339().replace('+', "%2B")
    );
    let (st, _, body) = get(format!(
        "/api/analytics/appointment-outcomes?group_by=specialty&{q}"
    ))
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body.contains(&a));
    let (st, ct, csv) = get(format!(
        "/api/analytics/appointment-outcomes?group_by=specialty&format=csv&{q}"
    ))
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(ct.starts_with("text/csv"));
    assert!(
        csv.starts_with("specialty,count\n") && csv.contains(&format!("{a},12")),
        "{csv}"
    );
    for bad in [
        "group_by=person_id",
        "group_by=",
        "group_by=specialty,specialty",
        "group_by=specialty,language,outcome,notice,time_of_day",
    ] {
        let (st, _, _) = get(format!("/api/analytics/appointment-outcomes?{bad}")).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    // Bad bands are refused.
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/analytics/demographics/{}", Uuid::new_v4()))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"age_band":"old","source":"ETL"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

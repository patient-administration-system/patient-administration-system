//! CORE-7: activity and performance reporting. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test builds its data inside a day of its own, years ahead, so other data cannot change its counts.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::appointment::{Appointment, CancellationReason};
use patient_administration_system::models::schedule::Slot;
use patient_administration_system::reports::GroupBy;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    day: DateTime<Utc>,
    a: Uuid,
    b: Uuid,
    appts: Vec<Appointment>,
}

/// Schedule A has ten slots on the day and seven appointments, in every outcome; schedule B has four free slots.
async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let d = 4000 + (Uuid::new_v4().as_u128() % 20_000) as i64;
    let day = (Utc::now() + Duration::days(d))
        .date_naive()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc();
    let a = common::new_schedule_for(
        &s.db,
        &format!("A{}", &Uuid::new_v4().simple().to_string()[..6]),
    )
    .await
    .id;
    let b = common::new_schedule_for(
        &s.db,
        &format!("B{}", &Uuid::new_v4().simple().to_string()[..6]),
    )
    .await
    .id;
    let mut slots_a = Vec::new();
    for i in 0..10 {
        let st = day + Duration::minutes(i * 20);
        slots_a.push(
            SlotRepository::create(&s.db, &Slot::new(a, st, st + Duration::minutes(20)))
                .await
                .unwrap()
                .id,
        );
    }
    for i in 0..4 {
        let st = day + Duration::minutes(i * 20);
        SlotRepository::create(&s.db, &Slot::new(b, st, st + Duration::minutes(20)))
            .await
            .unwrap();
    }
    let mut appts = Vec::new();
    for slot in slots_a.iter().take(7) {
        appts.push(
            s.scheduling
                .book_slot(*slot, Uuid::new_v4(), &sys())
                .await
                .unwrap(),
        );
    }
    // 0, 1 stay booked; 2 arrives; 3 is completed; 4 misses; 5 and 6 cancel (early, then late).
    s.scheduling.check_in(appts[2].id, &sys()).await.unwrap();
    s.scheduling.check_in(appts[3].id, &sys()).await.unwrap();
    s.scheduling.complete(appts[3].id, &sys()).await.unwrap();
    s.scheduling
        .mark_no_show(appts[4].id, &sys())
        .await
        .unwrap();
    for i in [5, 6] {
        s.scheduling
            .cancel(appts[i].id, CancellationReason::PatientRequest, &sys())
            .await
            .unwrap();
    }
    s.db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE appointments SET notice_days = 1 WHERE id = $1",
        [appts[6].id.into()],
    ))
    .await
    .unwrap();
    // Booked 10, 10, 20, 20, 30, 40 and 50 days before the visit.
    for (a, days) in appts.iter().zip([10, 10, 20, 20, 30, 40, 50]) {
        s.db.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE appointments SET created_at = start_datetime - make_interval(days => $2::int) WHERE id = $1",
            [a.id.into(), days.into()],
        ))
        .await
        .unwrap();
    }
    W {
        s,
        day,
        a,
        b,
        appts,
    }
}

impl W {
    fn window(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        (
            self.day - Duration::hours(8),
            self.day + Duration::hours(15),
        )
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn outcomes_reconcile_and_utilization_is_busy_over_total_per_schedule() {
    let w = world().await;
    let (from, to) = w.window();
    let r =
        w.s.reports
            .clinic_activity(Some(from), Some(to), GroupBy::Schedule)
            .await
            .unwrap();
    let a = r
        .rows
        .iter()
        .find(|x| x.key == w.a.to_string())
        .expect("schedule A");
    assert_eq!(
        (
            a.booked,
            a.attended,
            a.did_not_attend,
            a.cancelled_early,
            a.cancelled_late,
            a.total
        ),
        (2, 2, 1, 1, 1, 7)
    );
    assert_eq!(
        a.booked + a.attended + a.did_not_attend + a.cancelled_early + a.cancelled_late,
        a.total,
        "the outcomes of a row add up to its total"
    );
    // 10 slots; busy = 2 booked + 1 arrived + 1 completed (a miss and a cancel free the slot).
    assert_eq!((a.slots, a.busy_slots, a.utilization), (10, 4, 0.4));
    assert!(a.service.as_deref().unwrap().starts_with('A'));
    let b = r
        .rows
        .iter()
        .find(|x| x.key == w.b.to_string())
        .expect("schedule B has slots but no appointments");
    assert_eq!(
        (b.total, b.slots, b.busy_slots, b.utilization),
        (0, 4, 0, 0.0)
    );
    assert_eq!(
        r.total_appointments,
        r.rows.iter().map(|x| x.total).sum::<i64>()
    );
    assert_eq!(r.total_appointments, 7);
    assert_eq!((r.total_slots, r.busy_slots), (14, 4));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn grouping_by_day_gives_the_same_counts_in_one_row() {
    let w = world().await;
    let (from, to) = w.window();
    let r =
        w.s.reports
            .clinic_activity(Some(from), Some(to), GroupBy::Day)
            .await
            .unwrap();
    assert_eq!(r.rows.len(), 1);
    let d = &r.rows[0];
    assert_eq!(d.key, w.day.format("%Y-%m-%d").to_string());
    assert_eq!((d.total, d.slots, d.busy_slots), (7, 14, 4));
    assert_eq!(d.utilization, 0.286);
    assert_eq!(r.utilization, 0.286);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn lead_time_statistics_come_from_booking_to_visit() {
    let w = world().await;
    let (from, to) = w.window();
    let r =
        w.s.reports
            .clinic_activity(Some(from), Some(to), GroupBy::Day)
            .await
            .unwrap();
    let l = &r.lead_time_days;
    assert_eq!(l.n, 7);
    assert_eq!(l.median, 20.0);
    assert_eq!(l.p90, 50.0);
    assert_eq!(l.max, 50.0);
    assert_eq!(l.mean, 25.71);
    let _ = &w.appts;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_empty_range_gives_zeros_not_an_error() {
    let w = world().await;
    let empty_from = w.day + Duration::days(100);
    let r =
        w.s.reports
            .clinic_activity(
                Some(empty_from),
                Some(empty_from + Duration::days(1)),
                GroupBy::Schedule,
            )
            .await
            .unwrap();
    assert!(r.rows.is_empty());
    assert_eq!(
        (
            r.total_appointments,
            r.total_slots,
            r.busy_slots,
            r.utilization
        ),
        (0, 0, 0, 0.0)
    );
    assert_eq!(r.lead_time_days.n, 0);
    assert_eq!(r.lead_time_days.mean, 0.0);
    let csv = r.to_csv();
    assert_eq!(csv.lines().count(), 1, "just the header");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn open_pathway_waits_are_summarised_from_the_clock() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let mut ids = Vec::new();
    for weeks in [10, 20, 30] {
        let p = s
            .rtt
            .start(Uuid::new_v4(), format!("svc-{}", Uuid::new_v4()), &sys())
            .await
            .unwrap();
        s.db.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE rtt_clock_events SET event_at = now() - make_interval(weeks => $2::int) WHERE pathway_id = $1",
            [p.id.into(), weeks.into()],
        ))
        .await
        .unwrap();
        ids.push(p.id);
    }
    let r = s
        .reports
        .clinic_activity(None, None, GroupBy::Day)
        .await
        .unwrap();
    assert!(r.open_pathway_weeks.n >= 3);
    assert!(
        r.open_pathway_weeks.max >= 29.0,
        "{}",
        r.open_pathway_weeks.max
    ); // 30 weeks ago, less the milliseconds since
    assert!(
        r.breaching_pathways >= 1,
        "30 weeks is over the 18-week target"
    );
    for id in ids {
        for sql in [
            "DELETE FROM rtt_clock_events WHERE pathway_id = $1",
            "DELETE FROM rtt_pathways WHERE id = $1",
        ] {
            s.db.execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [id.into()],
            ))
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_gives_json_and_csv_without_identifiers_and_refuses_bad_input() {
    let w = world().await;
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
    let enc = |t: DateTime<Utc>| t.to_rfc3339().replace('+', "%2B");
    let q = format!("from={}&to={}", enc(from), enc(to));
    let (st, _, body) = get(format!(
        "/api/reports/clinic-activity?group_by=schedule&{q}"
    ))
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    for a in &w.appts {
        assert!(
            !body.contains(&a.id.to_string()) && !body.contains(&a.person_id.to_string()),
            "no identifier"
        );
    }
    let (st, ct, csv) = get(format!(
        "/api/reports/clinic-activity?group_by=schedule&format=csv&{q}"
    ))
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(ct.starts_with("text/csv"));
    assert!(csv.starts_with("key,service,booked,attended,did_not_attend,cancelled_early,cancelled_late,total,slots,busy_slots,utilization\n"));
    assert!(
        csv.contains(&format!("{},", w.a)) && csv.contains(",7,10,4,0.400"),
        "{csv}"
    );
    let (st, _, _) = get("/api/reports/clinic-activity?group_by=ward".into()).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, _) = get(format!(
        "/api/reports/clinic-activity?from={}&to={}",
        enc(to),
        enc(from)
    ))
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "to before from");
    let (st, _, _) = get(format!(
        "/api/reports/clinic-activity?from={}&to={}",
        enc(from - Duration::days(400)),
        enc(to)
    ))
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a range over a year");
}

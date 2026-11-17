//! db_bench — Criterion benchmarks against PostgreSQL: the booking transaction, the slot and calendar
//! queries, and a request through the whole REST router in process (PERF-1).
//!
//! These need `DATABASE_URL` (a migrated database, as for the `--ignored` tests) and, like those tests,
//! they fail rather than skip without one (NFR-10). Every row they make is under a schedule of their own
//! whose service type starts with `bench-`, so they can share a development database.
//!
//! ```sh
//! DATABASE_URL=postgres://pas:pas_dev_password@localhost:5442/pas cargo bench --bench db_bench
//! ```

#![cfg_attr(not(test), forbid(unsafe_code))]
use axum::body::Body;
use axum::http::Request;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::schedule::ScheduleRepository;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::appointment::CancellationReason;
use patient_administration_system::models::schedule::{Schedule, ScheduleOwner, Slot};
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;
use tower::ServiceExt;
use uuid::Uuid;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn state() -> AppState {
    let url = std::env::var("DATABASE_URL").expect(
        "DATABASE_URL is required: the database benchmarks need a migrated PostgreSQL (see podman/compose.yml)",
    );
    let db = connect(&url).await.expect("connect");
    AppState::new(db, Arc::new(InMemoryEventPublisher::new()))
}

async fn schedule(state: &AppState, name: &str) -> Schedule {
    let s = Schedule::new(
        ScheduleOwner::Practitioner(Uuid::new_v4()),
        format!("bench-{name}"),
    );
    ScheduleRepository::create(&state.db, &s)
        .await
        .expect("create schedule")
}

/// Each new slot is 30 minutes after the last, so no two overlap however many iterations run.
static NEXT_SLOT: AtomicI64 = AtomicI64::new(0);

async fn slot(state: &AppState, schedule_id: Uuid) -> Slot {
    let n = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
    let start = Timestamp::now()
        + patient_administration_system::time::days(400)
        + SignedDuration::from_mins(30 * n);
    SlotRepository::create(
        &state.db,
        &Slot::new(schedule_id, start, start + SignedDuration::from_mins(30)),
    )
    .await
    .expect("create slot")
}

/// Book a free slot (the transaction with its lock, clash check, audit row and outbox event), then cancel
/// it. Only the two calls are timed; making the slot is not.
fn bench_book_and_cancel(c: &mut Criterion) {
    let rt = runtime();
    let st = rt.block_on(state());
    let sched = rt.block_on(schedule(&st, "book"));
    let mut group = c.benchmark_group("db_booking");
    group.sample_size(30);
    group.bench_function("book_slot", |b| {
        b.to_async(&rt).iter_custom(|iters| {
            let st = st.clone();
            async move {
                let mut spent = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let s = slot(&st, sched.id).await;
                    let started = Instant::now();
                    let a = st
                        .scheduling
                        .book_slot(s.id, Uuid::new_v4(), &UserContext::system())
                        .await
                        .expect("book");
                    spent += started.elapsed();
                    black_box(a);
                }
                spent
            }
        });
    });
    group.bench_function("book_then_cancel", |b| {
        b.to_async(&rt).iter_custom(|iters| {
            let st = st.clone();
            async move {
                let mut spent = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let s = slot(&st, sched.id).await;
                    let started = Instant::now();
                    let a = st
                        .scheduling
                        .book_slot(s.id, Uuid::new_v4(), &UserContext::system())
                        .await
                        .expect("book");
                    st.scheduling
                        .cancel(
                            a.id,
                            CancellationReason::PatientRequest,
                            &UserContext::system(),
                        )
                        .await
                        .expect("cancel");
                    spent += started.elapsed();
                }
                spent
            }
        });
    });
    group.finish();
}

/// The free-slot search and the calendar range query, at a schedule holding 10, 100 and 1000 slots.
fn bench_queries(c: &mut Criterion) {
    let rt = runtime();
    let st = rt.block_on(state());
    let mut group = c.benchmark_group("db_queries");
    for n in [10_usize, 100, 1000] {
        let sched = rt.block_on(async {
            let sched = schedule(&st, &format!("query-{n}")).await;
            for _ in 0..n {
                slot(&st, sched.id).await;
            }
            sched
        });
        let from = Timestamp::now();
        let to = from + patient_administration_system::time::days(4000);
        group.bench_with_input(BenchmarkId::new("find_free_in_range", n), &n, |b, _| {
            b.to_async(&rt).iter(|| async {
                SlotRepository::find_free_in_range(&st.db, sched.id, from, to)
                    .await
                    .expect("query")
            });
        });
    }
    let from = Timestamp::now();
    group.bench_function("appointments_list_between_one_day", |b| {
        b.to_async(&rt).iter(|| async {
            AppointmentRepository::list_between(
                &st.db,
                from,
                from + patient_administration_system::time::days(1),
            )
            .await
            .expect("query")
        });
    });
    group.finish();
}

/// One request through the REST router in process: routing, extraction, the handler, the query and the
/// JSON envelope, without the network or the outer layers (`pas-load` measures those).
fn bench_router(c: &mut Criterion) {
    let rt = runtime();
    let st = rt.block_on(state());
    let sched = rt.block_on(schedule(&st, "router"));
    let booked = rt.block_on(async {
        let s = slot(&st, sched.id).await;
        st.scheduling
            .book_slot(s.id, Uuid::new_v4(), &UserContext::system())
            .await
            .expect("book")
    });
    let app = router(st.clone());
    let from = Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let to = (Timestamp::now() + patient_administration_system::time::days(1))
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let mut group = c.benchmark_group("router_in_process");
    for (name, uri) in [
        ("get_health", "/api/health".to_string()),
        (
            "get_appointment",
            format!("/api/appointments/{}", booked.id),
        ),
        (
            "list_appointments_one_day",
            format!("/api/appointments?from={from}&to={to}"),
        ),
        (
            "get_slot",
            format!("/api/slots/{}", booked.slot_id.unwrap_or_default()),
        ),
    ] {
        group.bench_function(name, |b| {
            b.to_async(&rt).iter(|| {
                let app = app.clone();
                let uri = uri.clone();
                async move {
                    let res = app
                        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
                        .await
                        .expect("response");
                    assert!(res.status().is_success(), "status {}", res.status());
                    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                        .await
                        .expect("body");
                    black_box(bytes)
                }
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_book_and_cancel, bench_queries, bench_router);
criterion_main!(benches);

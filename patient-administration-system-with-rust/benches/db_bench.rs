//! db_bench — Criterion benchmarks against PostgreSQL: the booking transaction, the slot and calendar
//! queries, a request through the whole REST router in process (PERF-1), the outbox dispatcher's query
//! against a backlog, and HL7 v2 de-duplication (PERF-3).
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
use patient_administration_system::db::repositories::outbox::OutboxRepository;
use patient_administration_system::db::repositories::schedule::ScheduleRepository;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::db::sql::{Conn, Stmt, Transact};
use patient_administration_system::hl7v2::ack::{AckCode, Inbound, ack_to};
use patient_administration_system::hl7v2::dedupe::guarded;
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

/// What the dispatcher asks on every tick (PERF-3, NFR-32): the next 100 due events, oldest first, each only
/// when no earlier event of its record is undelivered and no dead letter of its record is held. The backlog is
/// made inside a transaction that is rolled back, so the shared database's outbox is untouched; a database
/// that already has a backlog (a development one) is measured with it.
fn bench_outbox(c: &mut Criterion) {
    let rt = runtime();
    let st = rt.block_on(state());
    let mut group = c.benchmark_group("db_outbox_fetch_due");
    group.sample_size(20);
    for backlog in [1_000i64, 10_000, 100_000] {
        let txn = rt.block_on(async {
            let txn = st.db.begin_txn().await.expect("begin");
            // A record for every 4 events, so three in four wait behind an earlier one of their record.
            txn.run(Stmt::new(
                "INSERT INTO outbox_events (id, event_type, payload, published, at, retry_count) \
                 SELECT gen_random_uuid(), 'bench.event', \
                        jsonb_build_object('appointment_id', 'bench-' || (n / 4)), false, \
                        now() - make_interval(secs => ($1 - n)::double precision), 0 \
                 FROM generate_series(1, $1) AS n",
                [backlog.into()],
            ))
            .await
            .expect("seed the backlog");
            txn.run(Stmt::new("ANALYZE outbox_events", []))
                .await
                .expect("analyze");
            txn
        });
        group.bench_with_input(BenchmarkId::from_parameter(backlog), &backlog, |b, _| {
            b.to_async(&rt).iter(|| async {
                black_box(OutboxRepository::fetch_due(&txn, 100).await.expect("fetch"));
            });
        });
        // Rolled back on the runtime: sqlx returns the connection to the pool there.
        rt.block_on(async move { drop(txn) });
    }
    group.finish();
}

/// The work queue (TSC-1): the sync that runs every minute, when nothing has changed (the usual case), over a
/// backlog of open call tasks, and taking the next item of a queue. The seeded rows are marked by a due date in
/// 2999 and removed afterwards.
fn bench_work_queue(c: &mut Criterion) {
    use patient_administration_system::db::repositories::audit::UserContext;
    use patient_administration_system::work_queue::WorkQueueService;
    let rt = runtime();
    let st = rt.block_on(state());
    let svc = WorkQueueService::new(st.db.clone());
    let marker = "2999-01-01T00:00:00Z";
    let tidy = |db: &patient_administration_system::db::sql::Pool| {
        let db = db.clone();
        async move {
            db.run(Stmt::new(
                "DELETE FROM work_items WHERE kind = 'call' AND source_id IN \
                 (SELECT id FROM staff_call_tasks WHERE due_at = $1::timestamptz)",
                [marker.into()],
            ))
            .await
            .expect("tidy work items");
            db.run(Stmt::new(
                "DELETE FROM staff_call_tasks WHERE due_at = $1::timestamptz",
                [marker.into()],
            ))
            .await
            .expect("tidy call tasks");
        }
    };
    let mut group = c.benchmark_group("db_work_queue");
    group.sample_size(10);
    for backlog in [1_000i64, 10_000] {
        rt.block_on(async {
            tidy(&st.db).await;
            st.db
                .run(Stmt::new(
                    "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                     SELECT gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'silent', $2::timestamptz \
                     FROM generate_series(1, $1)",
                    [backlog.into(), marker.into()],
                ))
                .await
                .expect("seed call tasks");
            svc.sync(jiff::Timestamp::now()).await.expect("first sync");
        });
        group.bench_with_input(
            BenchmarkId::new("sync_unchanged", backlog),
            &backlog,
            |b, _| {
                b.to_async(&rt).iter(|| async {
                    black_box(svc.sync(jiff::Timestamp::now()).await.expect("sync"));
                });
            },
        );
    }
    let who = format!("bench-{}", uuid::Uuid::new_v4());
    let ctx = UserContext {
        user_id: Some(who.clone()),
        user_ip: None,
        user_agent: None,
    };
    group.bench_function("take_next_and_release", |b| {
        b.to_async(&rt).iter(|| async {
            if let Some(w) = svc.take_next("booking", &who, &ctx).await.expect("take") {
                svc.release(w.id, &who, &ctx).await.expect("release");
            }
        });
    });
    group.finish();
    rt.block_on(tidy(&st.db));
}

static NEXT_CONTROL_ID: AtomicI64 = AtomicI64::new(0);

/// HL7 v2 at most once (HUB-2): a new message's claim and finish, and a resent message answered from the
/// first ACK. Every row is under the sending application `BENCH`; the hourly purge removes them in time.
fn bench_hl7_dedupe(c: &mut Criterion) {
    let rt = runtime();
    let st = rt.block_on(state());
    let run = Uuid::new_v4().simple().to_string();
    let inbound = |n: i64| Inbound {
        sending_app: "BENCH".into(),
        sending_facility: run.clone(),
        receiving_app: "PAS".into(),
        receiving_facility: "HOSP".into(),
        event: "A01".into(),
        control_id: format!("CTL-{n}"),
        processing_id: "P".into(),
        version: "2.5".into(),
    };
    let accept = |i: &Inbound| ack_to(i, AckCode::Accept, None);
    let mut group = c.benchmark_group("db_hl7_dedupe");
    group.sample_size(30);
    group.bench_function("new_message", |b| {
        b.to_async(&rt).iter(|| {
            let i = inbound(NEXT_CONTROL_ID.fetch_add(1, Ordering::Relaxed));
            let db = st.db.clone();
            async move {
                let ack = accept(&i);
                black_box(guarded(&db, &i, async { ack }).await);
            }
        });
    });
    let resent = inbound(-1);
    rt.block_on(guarded(&st.db, &resent, async { accept(&resent) }));
    group.bench_function("resent_message", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                guarded(&st.db, &resent, async {
                    String::from("not run: a replay is answered from the first ACK")
                })
                .await,
            );
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_book_and_cancel,
    bench_queries,
    bench_router,
    bench_outbox,
    bench_hl7_dedupe,
    bench_work_queue
);
criterion_main!(benches);

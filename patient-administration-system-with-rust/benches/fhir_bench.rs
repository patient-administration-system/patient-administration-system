//! fhir_bench — Criterion benchmarks for the FHIR R5 mapping and the JSON it is sent as (PERF-1).
//!
//! A search returns a Bundle of mapped resources, so the cost that grows is the mapping and serialising of
//! many: this measures one of each resource, then a searchset of 10, 100 and 500 appointments (500 is the
//! largest page NFR-23 allows).

#![cfg_attr(not(test), forbid(unsafe_code))]
use chrono::{Duration, Utc};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use patient_administration_system::api::fhir::r5;
use patient_administration_system::models::appointment::Appointment;
use patient_administration_system::models::schedule::{Schedule, ScheduleOwner, Slot};
use std::hint::black_box;
use uuid::Uuid;

fn appointments(n: usize) -> Vec<Appointment> {
    let base = Utc::now();
    (0..n)
        .map(|i| {
            let start = base + Duration::minutes(i as i64 * 15);
            let mut a = Appointment::new(Uuid::new_v4(), start, start + Duration::minutes(15));
            a.slot_id = Some(Uuid::new_v4());
            a.reason = Some("follow-up".into());
            a
        })
        .collect()
}

fn bench_one_resource(c: &mut Criterion) {
    let a = appointments(1).remove(0);
    let schedule = Schedule::new(
        ScheduleOwner::Practitioner(Uuid::new_v4()),
        "outpatients".into(),
    );
    let start = Utc::now();
    let slot = Slot::new(schedule.id, start, start + Duration::minutes(30));
    c.bench_function("fhir_r5_appointment", |b| {
        b.iter(|| r5::appointment(black_box(&a)))
    });
    c.bench_function("fhir_r5_slot", |b| b.iter(|| r5::slot(black_box(&slot))));
    c.bench_function("fhir_r5_schedule", |b| {
        b.iter(|| r5::schedule(black_box(&schedule)));
    });
}

fn bench_searchset(c: &mut Criterion) {
    let mut group = c.benchmark_group("fhir_r5_searchset_appointments");
    for n in [10, 100, 500] {
        let rows = appointments(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(
            BenchmarkId::new("map_and_serialise", n),
            &rows,
            |b, rows| {
                b.iter(|| {
                    let resources = rows.iter().map(r5::appointment).collect();
                    let bundle = r5::searchset(
                        "https://pas.example.org/fhir",
                        "https://pas.example.org/fhir/Appointment",
                        resources,
                    );
                    serde_json::to_vec(black_box(&bundle)).unwrap()
                });
            },
        );
    }
    group.finish();
}

fn bench_native_json(c: &mut Criterion) {
    // The native REST API serialises the model directly; compare with the FHIR path above.
    let mut group = c.benchmark_group("rest_json_appointments");
    for n in [10, 100, 500] {
        let rows = appointments(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("serialise", n), &rows, |b, rows| {
            b.iter(|| serde_json::to_vec(black_box(rows)).unwrap());
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_one_resource,
    bench_searchset,
    bench_native_json
);
criterion_main!(benches);

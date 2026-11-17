//! rules_bench — Criterion benchmarks for the pure rules run over many rows at once (PERF-1).
//!
//! The no-show model runs for every upcoming appointment in its hourly pass, the triage verdict for every
//! attendance in a waits report, and the page cut on every list. Each is measured on one row, and the model
//! and the verdict over a day's worth of rows, so a regression in the per-row cost shows as a total.

#![cfg_attr(not(test), forbid(unsafe_code))]
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::noshow::{Inputs, Weights, compute};
use patient_administration_system::paging::cut;
use patient_administration_system::triage::targets::standing;
use std::hint::black_box;

fn inputs(n: usize) -> Vec<Inputs> {
    (0..n)
        .map(|i| Inputs {
            past_missed: (i % 4) as i64,
            past_attended: (i % 11) as i64,
            lead_days: (i % 120) as f64,
            age_band: None,
            weekday: i % 7,
            hour: 8 + (i % 10) as u32,
        })
        .collect()
}

fn bench_noshow(c: &mut Criterion) {
    let w = Weights::default();
    let one = inputs(1).remove(0);
    c.bench_function("noshow_compute_one", |b| {
        b.iter(|| compute(black_box(&one), black_box(&w)));
    });
    let mut group = c.benchmark_group("noshow_compute_pass");
    for n in [1_000, 10_000] {
        let rows = inputs(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &rows, |b, rows| {
            b.iter(|| rows.iter().map(|i| compute(i, &w).probability).sum::<f64>());
        });
    }
    group.finish();
}

fn bench_triage(c: &mut Criterion) {
    let now = Timestamp::now();
    let mut group = c.benchmark_group("triage_standing_report");
    for n in [100, 10_000] {
        let rows: Vec<_> = (0..n)
            .map(|i| {
                let arrival = now - SignedDuration::from_mins((i % 300) as i64);
                let seen =
                    (i % 3 != 0).then(|| arrival + SignedDuration::from_mins((i % 90) as i64));
                (arrival, seen, [0, 10, 60, 120, 240][i % 5])
            })
            .collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &rows, |b, rows| {
            b.iter(|| {
                rows.iter()
                    .filter(|(a, s, t)| standing(*a, *s, now, *t).is_past())
                    .count()
            });
        });
    }
    group.finish();
}

fn bench_page_cut(c: &mut Criterion) {
    let mut group = c.benchmark_group("paging_cut");
    for limit in [50_i64, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(limit), &limit, |b, &limit| {
            b.iter_batched(
                || (0..=limit).collect::<Vec<i64>>(),
                |rows| cut(rows, black_box(limit)),
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_noshow, bench_triage, bench_page_cut);
criterion_main!(benches);

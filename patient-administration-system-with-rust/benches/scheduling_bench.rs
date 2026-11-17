//! scheduling_bench — Criterion benchmarks for appointment overlap detection.
//!
//! Measures `TimeRange::overlaps` across a linear scan at increasing range
//! counts to detect regressions in the inner loop used by appointment
//! conflict detection.

#![cfg_attr(not(test), forbid(unsafe_code))]
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::models::TimeRange;
use std::hint::black_box;

fn make_ranges(count: usize) -> Vec<TimeRange> {
    let mut ranges = Vec::with_capacity(count);
    let base = Timestamp::now();
    for i in 0..count {
        let start = base + SignedDuration::from_mins(i as i64 * 30);
        let end = start + SignedDuration::from_mins(30);
        ranges.push(TimeRange { start, end });
    }
    ranges
}

fn bench_overlap_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("timerange_overlap_scan");
    for size in [16, 64, 256, 1024].iter() {
        let ranges = make_ranges(*size);
        let target = TimeRange {
            start: Timestamp::now() + SignedDuration::from_mins(100),
            end: Timestamp::now() + SignedDuration::from_mins(130),
        };
        group.bench_with_input(BenchmarkId::from_parameter(size), &ranges, |b, ranges| {
            b.iter(|| {
                let mut hits = 0;
                for r in ranges.iter() {
                    if r.overlaps(black_box(&target)) {
                        hits += 1;
                    }
                }
                black_box(hits)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_overlap_scan);
criterion_main!(benches);

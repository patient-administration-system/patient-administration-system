//! hl7v2_bench — Criterion benchmarks for the HL7 v2 path every inbound message takes (PERF-1).
//!
//! Parsing, a batch, the PID to person translation, encoding a reply, and escaping. The inputs are the
//! files in `examples/`, so a benchmark reads what a sending system would send.

#![cfg_attr(not(test), forbid(unsafe_code))]
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use patient_administration_system::hl7v2::batch::parse_batch;
use patient_administration_system::hl7v2::encoder::encode_message;
use patient_administration_system::hl7v2::escape::{escape_value, unescape_value};
use patient_administration_system::hl7v2::parser::parse_message;
use patient_administration_system::hl7v2::pid_translation::{
    parse_hl7_datetime, pid_to_person_create,
};
use std::hint::black_box;

const A01: &str = include_str!("../examples/hl7-adt-a01.txt");
const A02: &str = include_str!("../examples/hl7-adt-a02.txt");
const A03: &str = include_str!("../examples/hl7-adt-a03.txt");
const A28: &str = include_str!("../examples/hl7-adt-a28.txt");
const BATCH: &str = include_str!("../examples/hl7-batch-adt.txt");

/// A message with `n` repeated OBX-like note segments, to see how parsing grows with size.
fn long_message(n: usize) -> String {
    let mut s = A01.trim_end().to_string();
    for i in 0..n {
        s.push_str(&format!(
            "\nNTE|{i}||A free-text note with components^and^repeats~and escapes \\F\\ \\S\\ number {i}"
        ));
    }
    s
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("hl7v2_parse");
    for (name, msg) in [
        ("adt_a01", A01),
        ("adt_a02", A02),
        ("adt_a03", A03),
        ("adt_a28", A28),
    ] {
        group.throughput(Throughput::Bytes(msg.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), msg, |b, m| {
            b.iter(|| parse_message(black_box(m)).unwrap());
        });
    }
    for n in [10, 100, 1000] {
        let msg = long_message(n);
        group.throughput(Throughput::Bytes(msg.len() as u64));
        group.bench_with_input(BenchmarkId::new("segments", n), &msg, |b, m| {
            b.iter(|| parse_message(black_box(m)).unwrap());
        });
    }
    group.finish();
}

fn bench_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("hl7v2_batch");
    group.throughput(Throughput::Bytes(BATCH.len() as u64));
    group.bench_function("parse_batch_example", |b| {
        b.iter(|| parse_batch(black_box(BATCH)).unwrap());
    });
    group.finish();
}

fn bench_translate_and_encode(c: &mut Criterion) {
    let msg = parse_message(A01).unwrap();
    let pid = msg
        .segments
        .iter()
        .find(|s| s.name == "PID")
        .unwrap()
        .clone();
    c.bench_function("hl7v2_pid_to_person_create", |b| {
        b.iter(|| pid_to_person_create(black_box(&pid)));
    });
    c.bench_function("hl7v2_encode_message_a01", |b| {
        b.iter(|| encode_message(black_box(&msg)));
    });
    c.bench_function("hl7v2_parse_datetime", |b| {
        b.iter(|| parse_hl7_datetime(black_box("20260523120000+0100")));
    });
}

fn bench_escape(c: &mut Criterion) {
    let plain = "A plain note with nothing to escape in it at all, as most fields are";
    let busy = "Pipes | carets ^ tildes ~ ampersands & and backslashes \\ in one note";
    let escaped = escape_value(busy);
    c.bench_function("hl7v2_escape_plain", |b| {
        b.iter(|| escape_value(black_box(plain)))
    });
    c.bench_function("hl7v2_escape_busy", |b| {
        b.iter(|| escape_value(black_box(busy)))
    });
    c.bench_function("hl7v2_unescape_busy", |b| {
        b.iter(|| unescape_value(black_box(&escaped)));
    });
}

criterion_group!(
    benches,
    bench_parse,
    bench_batch,
    bench_translate_and_encode,
    bench_escape
);
criterion_main!(benches);

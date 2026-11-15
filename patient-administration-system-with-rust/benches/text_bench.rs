//! text_bench — Criterion benchmarks for the small text functions on every request or message (PERF-1).
//!
//! Identifier checks (on every patient write through the proxy), phone normalisation, SMS length (on
//! every template edit and send), template rendering, path redaction (on every request's log line) and
//! reply-link tokens.

#![cfg_attr(not(test), forbid(unsafe_code))]
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use patient_administration_system::communication::sms_length::analyze;
use patient_administration_system::communication::templates::sample_sms;
use patient_administration_system::contact::normalise_e164;
use patient_administration_system::identifiers::{check, check_person_identifiers};
use patient_administration_system::link_token::{hash, new_token};
use patient_administration_system::observability::redact_path;
use std::hint::black_box;

const NHS_SYSTEM: &str = "https://fhir.nhs.uk/Id/nhs-number";

fn bench_identifiers(c: &mut Criterion) {
    let mut group = c.benchmark_group("identifier_check");
    for (name, value) in [
        ("valid", "943 476 5919"),
        ("bad_check_digit", "9434765918"),
        ("wrong_length", "12345"),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), value, |b, v| {
            // A failing value is an `Err`: the refusal is what is timed.
            b.iter(|| check(black_box(NHS_SYSTEM), black_box(v)).is_ok());
        });
    }
    group.finish();

    // A patient body as the proxy sees it: several identifiers, one with a scheme, others without.
    let body: serde_json::Value = serde_json::json!({
        "identifiers": [
            {"system": NHS_SYSTEM, "value": "9434765919"},
            {"system": "urn:example:mrn", "value": "MRN-0001"},
            {"system": "urn:example:passport", "value": "X1234567"}
        ],
        "name": {"text": "Example Person"}
    });
    let fields = body.as_object().unwrap().clone();
    c.bench_function("check_person_identifiers_three", |b| {
        b.iter(|| check_person_identifiers(black_box(&fields)).unwrap());
    });
}

fn bench_phone(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalise_e164");
    for (name, raw, cc) in [
        ("already_e164", "+447700900123", None),
        ("national_with_country", "07700 900123", Some("44")),
        ("punctuated", "+1 (555) 010-0199", None),
    ] {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(raw, cc),
            |b, (r, cc)| {
                b.iter(|| normalise_e164(black_box(r), black_box(*cc)).unwrap());
            },
        );
    }
    group.finish();
}

fn bench_sms(c: &mut Criterion) {
    let mut group = c.benchmark_group("sms_length");
    let gsm =
        "Your appointment is on Tuesday 3 November at 09:30. Reply 1 to confirm, 2 to rebook.";
    let welsh = "Mae eich apwyntiad ddydd Mawrth 3 Tachwedd am 09:30. Atebwch 1 i gadarnhau, 2 i ail-drefnu ŵ.";
    let arabic = "موعدك يوم الثلاثاء 3 نوفمبر الساعة 09:30. أجب بـ 1 للتأكيد أو 2 لإعادة الحجز.";
    let long = gsm.repeat(8);
    for (name, text) in [
        ("gsm", gsm),
        ("welsh_ucs2", welsh),
        ("arabic_ucs2", arabic),
        ("gsm_long", long.as_str()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), text, |b, t| {
            b.iter(|| analyze(black_box(t)));
        });
    }
    group.finish();

    let body = "Hello {{ patient.given_name | default(value=\"\") }}, your appointment is {{ when }}. \
                Reply 1 to confirm.";
    c.bench_function("template_sample_sms_render", |b| {
        b.iter(|| sample_sms(black_box(body), black_box("en-001")));
    });
}

fn bench_redact_and_tokens(c: &mut Criterion) {
    let mut group = c.benchmark_group("redact_path");
    for (name, path) in [
        ("no_id", "/api/health"),
        (
            "one_uuid",
            "/api/appointments/7c9e6679-7425-40de-944b-e07fc1f90ae7/cancel",
        ),
        (
            "token_and_uuid",
            "/r/4f1d2c3b4a5968778695a4b3c2d1e0f1/7c9e6679-7425-40de-944b-e07fc1f90ae7",
        ),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), path, |b, p| {
            b.iter(|| redact_path(black_box(p)));
        });
    }
    group.finish();

    c.bench_function("link_token_new", |b| b.iter(new_token));
    let token = new_token();
    c.bench_function("link_token_hash", |b| b.iter(|| hash(black_box(&token))));
}

criterion_group!(
    benches,
    bench_identifiers,
    bench_phone,
    bench_sms,
    bench_redact_and_tokens
);
criterion_main!(benches);

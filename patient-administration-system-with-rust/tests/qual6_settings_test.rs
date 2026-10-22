//! QUAL-6: the environment is read in one place, checked at start-up, and a bad value stops the server with a
//! message that names it. No database needed.

use patient_administration_system::config::env::{Kind, VARS};
use std::path::{Path, PathBuf};
use std::process::Command;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The production part of a source file: everything before its test module.
fn production_part(text: &str) -> &str {
    text.find("#[cfg(test)]").map_or(text, |i| &text[..i])
}

fn sources() -> Vec<(PathBuf, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files
        .into_iter()
        .filter(|p| {
            let rel = p.strip_prefix(&src).expect("under src");
            !rel.starts_with("config") && !rel.starts_with("bin")
        })
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("read");
            (p, text)
        })
        .collect()
}

#[test]
fn nothing_outside_config_reads_the_environment() {
    let mut offenders = Vec::new();
    for (path, text) in sources() {
        let prod = production_part(&text);
        for pat in ["env::var(", "env::var_os(", "env::vars("] {
            if prod.contains(pat) {
                offenders.push(format!("{} uses {pat}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "read the environment through config::env: {offenders:?}"
    );
}

/// Every name passed to `env::flag`, `env::int` or `env::float` is registered, with the matching kind.
#[test]
fn every_accessor_name_is_registered_with_the_right_kind() {
    let mut checked = 0;
    for (path, text) in sources() {
        let prod = production_part(&text);
        for (accessor, want) in [("flag", "Flag"), ("int", "Int"), ("float", "Float")] {
            let needle = format!("env::{accessor}(");
            let mut rest = prod;
            while let Some(at) = rest.find(&needle) {
                let after = rest[at + needle.len()..].trim_start();
                rest = &rest[at + needle.len()..];
                let Some(name) = after.strip_prefix('"').and_then(|r| r.split('"').next()) else {
                    continue; // a variable, not a literal: covered where it is built
                };
                let var = VARS.iter().find(|v| v.name == name).unwrap_or_else(|| {
                    panic!("{} reads {name}, which is not in VARS", path.display())
                });
                let got = match var.kind {
                    Kind::Flag { .. } => "Flag",
                    Kind::Int { .. } => "Int",
                    Kind::Float { .. } => "Float",
                    Kind::Json => "Json",
                    Kind::TimeZone => "TimeZone",
                };
                assert_eq!(
                    got,
                    want,
                    "{} reads {name} with env::{accessor}",
                    path.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 15, "the scan found only {checked} reads");
}

fn start(env: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_patient-administration-system"));
    cmd.env_clear()
        .env("DATABASE_URL", "postgres://nobody:nothing@127.0.0.1:1/none");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run the server binary");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn a_bad_value_stops_the_server_and_names_the_variable() {
    let (ok, text) = start(&[
        ("PAS_TOLERANCE_CHECK_MINUTES", "abc"),
        ("PAS_NOSHOW_SCORE", "maybe"),
    ]);
    assert!(!ok, "the server must not start: {text}");
    assert!(text.contains("invalid environment"), "{text}");
    assert!(text.contains("PAS_TOLERANCE_CHECK_MINUTES"), "{text}");
    assert!(
        text.contains("PAS_NOSHOW_SCORE"),
        "all problems at once: {text}"
    );
}

#[test]
fn good_values_get_past_the_check_and_fail_later_on_the_database() {
    // `TRUE` and `1` are valid switches; the server then fails only because no database answers.
    let (ok, text) = start(&[
        ("PAS_NOSHOW_SCORE", "TRUE"),
        ("PAS_BACKFILL_OFFERS", "1"),
        ("PAS_TOLERANCE_CHECK_MINUTES", "0"),
        // 0 turns the rate limiter off; it must not stop the server starting.
        ("PAS_RATE_LIMIT_RPM", "0"),
        ("PAS_RATE_LIMIT_BURST", "0"),
    ]);
    assert!(!ok);
    assert!(!text.contains("invalid environment"), "{text}");
}

//! QUAL-52: the domain crate (`domain/`, `patient-administration-system-domain`) holds the business rules and
//! nothing that serves the web, stores data or calls another system.
//!
//! Two checks, both on text, so they need no database: its manifest names no web framework, HTTP client,
//! database driver, async runtime or template engine, and its sources name none of their paths. The server
//! re-exports the domain modules at their old paths; `the_server_still_reaches_the_moved_modules` proves the
//! paths the rest of the code and the tests use still resolve.

use std::path::{Path, PathBuf};

/// Crates that belong to the server, not to the rules.
const FORBIDDEN: &[&str] = &[
    "axum",
    "axa",
    "tower",
    "tower-http",
    "hyper",
    "http",
    "reqwest",
    "sqlx",
    "axa-postgres-sqlx",
    "tokio",
    "async-trait",
    "tera",
    "rustls",
    "tokio-rustls",
    "opentelemetry",
    "tracing-subscriber",
    "patient-administration-system",
    "patient-administration-system-migrations",
];

fn domain_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("domain")
}

/// The dependency names in the `[dependencies]` table of the domain manifest.
fn dependencies() -> Vec<String> {
    let manifest = std::fs::read_to_string(domain_dir().join("Cargo.toml")).unwrap();
    let mut names = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[dependencies]";
            continue;
        }
        if inside
            && !line.starts_with('#')
            && let Some((name, _)) = line.split_once('=')
        {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_domain_manifest_names_no_server_crate() {
    let deps = dependencies();
    assert!(
        deps.iter().any(|d| d == "serde"),
        "the manifest was not read: {deps:?}"
    );
    let found: Vec<&String> = deps
        .iter()
        .filter(|d| FORBIDDEN.contains(&d.as_str()) || d.starts_with("axa-"))
        .collect();
    assert!(
        found.is_empty(),
        "the domain crate is for business rules; move the code that needs {found:?} to the server crate"
    );
}

#[test]
fn the_domain_sources_name_no_server_crate() {
    let mut files = Vec::new();
    rust_files(&domain_dir().join("src"), &mut files);
    assert!(files.len() > 10, "found only {} files", files.len());
    let mut found = Vec::new();
    for f in &files {
        for (n, line) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for c in FORBIDDEN {
                let path = format!("{}::", c.replace('-', "_"));
                if code.contains(&path) && !code.contains(&format!("_{path}")) {
                    found.push(format!("{}:{}: {code}", f.display(), n + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_server_still_reaches_the_moved_modules() {
    use patient_administration_system as pas;
    let _: fn(&str) -> Option<String> = pas::config::env::text;
    let _ = pas::models::waitlist::Priority::Routine;
    assert!(pas::identifiers::by_system("urn:not-a-scheme").is_none());
    let _ = pas::privacy::mask_value("07700900123", 3);
    let _ = pas::communication::sms_length::analyze("hello");
    let _ = pas::inbound::messages::Lang::En;
    let _: &str = pas::suspicions::profile::SETTING;
    // A rule's refusal keeps its server variant, so its HTTP status does not change.
    let e: pas::Error = patient_administration_system_domain::Error::validation("x").into();
    assert!(matches!(e, pas::Error::Validation(_)));
}

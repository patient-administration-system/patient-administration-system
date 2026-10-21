//! QUAL-33: every `PAS_*` switch the server reads is documented for operators. No database needed.
//!
//! The switches are the `"PAS_…"` string literals in `src/` outside unit tests; the operator documents are
//! `operations/deployment.md` and `operations/runbook.md`. A new switch fails this test until it is
//! written down there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `"PAS_NAME"` literal: a quote, `PAS_`, upper-case letters, digits or underscores, a quote.
fn switches(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (i, _) in text.match_indices("\"PAS_") {
        let rest = &text[i + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        if rest[end..].starts_with('"') {
            found.insert(rest[..end].to_string());
        }
    }
    found
}

#[test]
fn every_pas_switch_in_the_server_is_in_the_operations_documents() {
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src"), &mut files);
    let mut used = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        // Unit tests below a file's first `#[cfg(test)]` may name made-up switches.
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        used.extend(switches(code));
    }
    assert!(
        used.len() > 40,
        "found only {} switches: {used:?}",
        used.len()
    );

    let ops = crate_dir().join("../operations");
    let docs = ["deployment.md", "runbook.md"]
        .iter()
        .map(|d| std::fs::read_to_string(ops.join(d)).unwrap())
        .collect::<String>();
    let missing: Vec<_> = used.iter().filter(|s| !docs.contains(s.as_str())).collect();
    assert!(
        missing.is_empty(),
        "undocumented switches (add them to operations/deployment.md): {missing:?}"
    );
}

#[test]
fn the_scanner_takes_whole_quoted_names_only() {
    let found = switches(r#"env("PAS_A_1") "PAS_B" PAS_C "PAS_lower" "PAS_D.x""#);
    assert_eq!(found.into_iter().collect::<Vec<_>>(), ["PAS_A_1", "PAS_B"]);
}

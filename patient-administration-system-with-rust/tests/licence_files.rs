//! DTAC-3: every licence a manifest names has its text in `LICENSE/`, and the texts are real.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// SPDX identifiers in an expression such as `MIT OR Apache-2.0 OR GPL-3.0-only`.
fn ids(expression: &str) -> Vec<String> {
    expression
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|t| !t.is_empty() && !matches!(*t, "OR" | "AND" | "WITH"))
        .map(String::from)
        .collect()
}

fn manifest_licences() -> BTreeSet<String> {
    let root = repo_root();
    let mut found = BTreeSet::new();
    for manifest in [
        "patient-administration-system-with-rust/Cargo.toml",
        "patient-administration-system-frontend/Cargo.toml",
    ] {
        let text = fs::read_to_string(root.join(manifest)).expect(manifest);
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with("license = "))
            .unwrap_or_else(|| panic!("{manifest} declares no license"));
        found.extend(ids(line.split('"').nth(1).expect("quoted license")));
    }
    let package =
        fs::read_to_string(root.join("patient-administration-system-ui-with-svelte/package.json"))
            .unwrap();
    let json: serde_json::Value = serde_json::from_str(&package).unwrap();
    found.extend(ids(json["license"].as_str().expect("package.json license")));
    found
}

#[test]
fn every_declared_licence_has_a_text() {
    let dir = repo_root().join("LICENSE");
    for id in manifest_licences() {
        let path = dir.join(format!("{id}.txt"));
        let text =
            fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
        assert!(text.len() > 500, "{} looks empty", path.display());
    }
}

#[test]
fn licence_index_lists_every_text() {
    let index = fs::read_to_string(repo_root().join("LICENSE.md")).unwrap();
    for entry in fs::read_dir(repo_root().join("LICENSE")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            index.contains(&format!("LICENSE/{name}")),
            "LICENSE.md does not link {name}"
        );
    }
}

#[test]
fn the_expression_parser_handles_or_and_parentheses() {
    assert_eq!(
        ids("MIT OR (Apache-2.0 AND BSD-3-Clause)"),
        ["MIT", "Apache-2.0", "BSD-3-Clause"]
    );
}

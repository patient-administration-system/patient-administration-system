//! DTAC-3: every licence a manifest declares has its text in the repository, and the security policy exists.
//! No database needed. The root `LICENSE/` directory holds one `<SPDX id>.txt` per licence.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The SPDX identifiers in an expression made only of `OR`s, which is all this repository uses.
fn ids(expr: &str) -> Vec<String> {
    expr.split(" OR ").map(|s| s.trim().to_string()).collect()
}

fn cargo_license(path: &str) -> String {
    let text = std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines()
        .find_map(|l| {
            l.strip_prefix("license = \"")
                .and_then(|r| r.strip_suffix('"'))
        })
        .unwrap_or_else(|| panic!("{path} declares no licence"))
        .to_string()
}

#[test]
fn test_every_declared_licence_has_a_text_file() {
    let mut declared = vec![
        cargo_license("patient-administration-system-with-rust/Cargo.toml"),
        cargo_license("patient-administration-system-frontend/Cargo.toml"),
    ];
    let pkg = std::fs::read_to_string(
        repo().join("patient-administration-system-ui-with-svelte/package.json"),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&pkg).unwrap();
    declared.push(
        v["license"]
            .as_str()
            .expect("the UI declares a licence")
            .to_string(),
    );

    for expr in declared {
        for id in ids(&expr) {
            let file = repo().join("LICENSE").join(format!("{id}.txt"));
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|_| panic!("{id} is declared but {} is missing", file.display()));
            assert!(
                text.trim().len() > 200,
                "{id}: the licence text looks empty"
            );
        }
    }
}

#[test]
fn test_the_licence_summary_names_every_text_file() {
    let md = std::fs::read_to_string(repo().join("LICENSE.md")).unwrap();
    for entry in std::fs::read_dir(repo().join("LICENSE")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            md.contains(&name),
            "LICENSE.md does not mention LICENSE/{name}"
        );
    }
}

#[test]
fn test_there_is_a_security_policy_with_a_way_to_report() {
    let s = std::fs::read_to_string(repo().join("SECURITY.md")).expect("SECURITY.md");
    assert!(
        s.to_lowercase().contains("report"),
        "SECURITY.md says how to report a vulnerability"
    );
    assert!(s.len() > 500);
}

//! NEW-25: the PAS records a triage category and never decides one. No database needed.
//!
//! The Manchester Triage System's flowcharts and discriminators are licensed content and a clinical
//! decision (plan §11). This fails if the server's code starts to name a discriminator outside a comment,
//! so adding triage logic is a deliberate act that needs the licence and a Clinical Safety Officer first.

use std::path::{Path, PathBuf};

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

#[test]
fn the_server_code_names_no_triage_discriminator() {
    let mut files = Vec::new();
    rust_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 50, "found only {} files", files.len());
    let mut found = Vec::new();
    for f in &files {
        for (n, line) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            let code = line.trim_start();
            if !code.starts_with("//") && code.to_lowercase().contains("discriminator") {
                found.push(format!("{}:{}: {code}", f.display(), n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "triage logic needs a licence and a CSO first: {found:#?}"
    );
}

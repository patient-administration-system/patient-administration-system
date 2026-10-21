//! QUAL-8: a step whose failure must not pass unseen is never thrown away with `let _ =`. No database needed.

use std::path::{Path, PathBuf};

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

/// The statement after `let _ = ` up to its `;`, whitespace removed.
fn statements_after_let_underscore(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("let _ = ") {
        let body = &rest[at + "let _ = ".len()..];
        let mut depth = 0i32;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        out.push(body[..end].split_whitespace().collect::<String>());
        rest = &body[end..];
    }
    out
}

#[test]
fn audit_outbox_and_event_results_are_never_discarded() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut offenders = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("read");
        let prod = text
            .find("#[cfg(test)]")
            .map_or(text.as_str(), |i| &text[..i]);
        for stmt in statements_after_let_underscore(prod) {
            let discarded = [
                "AuditLogRepository::log",
                "OutboxRepository::publish",
                "self.publisher.publish",
                "self.scheduling.publish_event",
            ];
            if discarded.iter().any(|d| stmt.starts_with(d)) {
                offenders.push(format!(
                    "{}: let _ = {}",
                    path.display(),
                    &stmt[..stmt.len().min(70)]
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "log the failure with LogErr (log_error or log_warn) instead: {offenders:#?}"
    );
}

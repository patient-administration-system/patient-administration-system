//! QUAL-7: every static SQL statement in the server prepares against the migrated schema, so a misspelt
//! column, a renamed table or an impossible type shows up here and not on the first request that needs it.
//! Needs PostgreSQL: `cargo test -- --ignored`. Statements built with `format!` are not covered (17 at the time
//! of writing); the others are found by reading the source.

mod common;

use sea_orm::{ConnectionTrait, TransactionTrait};
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

/// Read a Rust string literal that starts at `s[0] == '"'` (or `r"` / `r#"`). Returns its value and how many
/// bytes it took.
fn string_literal(s: &str) -> Option<(String, usize)> {
    if let Some(rest) = s.strip_prefix("r#\"") {
        let end = rest.find("\"#")?;
        return Some((rest[..end].to_string(), 3 + end + 2));
    }
    if let Some(rest) = s.strip_prefix("r\"") {
        let end = rest.find('"')?;
        return Some((rest[..end].to_string(), 2 + end + 1));
    }
    let mut chars = s.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, i + 1)),
            '\\' => match chars.next()?.1 {
                '\n' => {
                    // A line continuation swallows the newline and the next line's indentation.
                    let rest = &s[i + 2..];
                    let skip = rest.len() - rest.trim_start().len();
                    for _ in 0..skip {
                        chars.next();
                    }
                }
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '\\' => out.push('\\'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// `(file, line, sql)` for every `from_sql_and_values(DbBackend::Postgres, "<literal>"` in production code.
fn static_statements() -> (Vec<(PathBuf, usize, String)>, usize) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let (mut found, mut dynamic) = (Vec::new(), 0);
    for path in files {
        let text = std::fs::read_to_string(&path).expect("read");
        let prod = text
            .find("#[cfg(test)]")
            .map_or(text.as_str(), |i| &text[..i]);
        let needle = "from_sql_and_values(";
        let mut from = 0;
        while let Some(at) = prod[from..].find(needle) {
            let start = from + at + needle.len();
            from = start;
            let rest = prod[start..].trim_start();
            let Some(rest) = rest.strip_prefix("DbBackend::Postgres,") else {
                continue;
            };
            let rest = rest.trim_start();
            match string_literal(rest) {
                Some((sql, _)) => {
                    let line = prod[..start].matches('\n').count() + 1;
                    found.push((path.clone(), line, sql));
                }
                None => dynamic += 1,
            }
        }
    }
    (found, dynamic)
}

#[test]
fn the_literal_reader_handles_escapes_and_continuations() {
    let (v, n) = string_literal("\"SELECT 1 \\\n       FROM t\" , x").expect("a literal");
    assert_eq!(v, "SELECT 1 FROM t");
    assert_eq!(n, "\"SELECT 1 \\\n       FROM t\"".len());
    assert_eq!(string_literal("\"a\\\"b\"").expect("a literal").0, "a\"b");
    assert_eq!(string_literal("r#\"x \"y\"\"#").expect("raw").0, "x \"y\"");
    assert!(string_literal("format!(\"x\")").is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_static_sql_statement_prepares() {
    let db = common::build_state(&common::database_url().unwrap())
        .await
        .db;
    let (statements, dynamic) = static_statements();
    assert!(
        statements.len() > 150,
        "the scan found only {} statements",
        statements.len()
    );
    let (mut prepared, mut ambiguous, mut failures) = (0, 0, Vec::new());
    for (n, (path, line, sql)) in statements.iter().enumerate() {
        let first = sql
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if !["SELECT", "INSERT", "UPDATE", "DELETE", "WITH"].contains(&first.as_str()) {
            continue; // DDL and advisory-lock calls are not prepared
        }
        // One transaction (so one connection) per statement. A prepared statement belongs to the connection and
        // survives a rollback, so it has its own name and is dropped again.
        let name = format!("qual7_statement_{n}");
        let txn = db.begin().await.expect("begin");
        let result = txn
            .execute_unprepared(&format!("PREPARE {name} AS {sql}"))
            .await;
        if result.is_ok() {
            txn.execute_unprepared(&format!("DEALLOCATE {name}"))
                .await
                .expect("deallocate");
        }
        let _ = txn.rollback().await;
        match result {
            Ok(_) => prepared += 1,
            // A parameter whose type only the bound value decides (`$1 IS NULL`): cannot be checked here.
            Err(e)
                if e.to_string()
                    .contains("could not determine data type of parameter") =>
            {
                ambiguous += 1
            }
            Err(e) => failures.push(format!("{}:{line}: {e}\n    {sql}", path.display())),
        }
    }
    eprintln!(
        "{prepared} statements prepared, {ambiguous} have a parameter whose type is only known when bound, \
         {dynamic} are built with format! and were not checked"
    );
    assert!(
        failures.is_empty(),
        "these statements do not prepare against the migrated schema:\n{}",
        failures.join("\n")
    );
    assert!(prepared > 120, "only {prepared} statements were prepared");
}

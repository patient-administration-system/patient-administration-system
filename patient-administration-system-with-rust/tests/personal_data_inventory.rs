//! NFR-12: `governance/personal-data-inventory.csv` classifies EVERY column of the database, and
//! stays true to it. Adding a column without classifying it (or classifying one that does not
//! exist) fails this test, so personal data cannot arrive by accident.
//!
//! Needs PostgreSQL with the migrations applied: `DATABASE_URL=… cargo test -- --ignored`.

mod common;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const CATEGORIES: [&str; 8] = [
    "identifier",
    "staff-identifier",
    "health",
    "preference",
    "free-text",
    "financial",
    "staff-trace",
    "none",
];

type Row = (String, String, String);

fn inventory() -> Vec<Row> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("governance/personal-data-inventory.csv");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("table,column,category,purpose,retention_class"),
        "header"
    );
    lines
        .map(|l| {
            let f: Vec<&str> = l.splitn(5, ',').collect();
            assert_eq!(f.len(), 5, "malformed row: {l}");
            (f[0].to_string(), f[1].to_string(), f[2].to_string())
        })
        .collect()
}

#[test]
fn inventory_rows_are_well_formed_and_unique() {
    let rows = inventory();
    let mut seen = BTreeSet::new();
    for (t, c, cat) in &rows {
        assert!(
            CATEGORIES.contains(&cat.as_str()),
            "{t}.{c}: unknown category {cat:?}"
        );
        assert!(
            seen.insert((t.clone(), c.clone())),
            "{t}.{c} is listed twice"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_database_column_is_classified_and_no_stale_row_remains() {
    let url = common::database_url().unwrap();
    let db = patient_administration_system::db::connect(&url)
        .await
        .expect("connect");
    let rows = db
        .query_all(Statement::from_string(
            DbBackend::Postgres,
            "SELECT table_name, column_name FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name NOT LIKE 'seaql%'"
                .to_string(),
        ))
        .await
        .unwrap();
    let actual: BTreeSet<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.try_get("", "table_name").unwrap(),
                r.try_get("", "column_name").unwrap(),
            )
        })
        .collect();
    let listed: BTreeSet<(String, String)> =
        inventory().into_iter().map(|(t, c, _)| (t, c)).collect();

    let unclassified: Vec<_> = actual.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&actual).collect();
    assert!(
        unclassified.is_empty(),
        "columns not in governance/personal-data-inventory.csv (classify them): {unclassified:?}"
    );
    assert!(
        stale.is_empty(),
        "inventory rows for columns that no longer exist: {stale:?}"
    );
}

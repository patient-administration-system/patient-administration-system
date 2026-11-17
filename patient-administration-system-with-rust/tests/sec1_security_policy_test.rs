//! SEC-1: `SECURITY.md` is a formal policy, and it cannot quietly go out of date. No database.
use jiff::civil::Date;
use std::path::PathBuf;

fn policy() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join("SECURITY.md")).expect("SECURITY.md")
}

/// The date in the table row `| <label> | <text with a YYYY-MM-DD> |`; the last one when there are two.
fn date_in_row(policy: &str, label: &str) -> Date {
    let row = policy
        .lines()
        .find(|l| l.starts_with(&format!("| {label} |")))
        .unwrap_or_else(|| panic!("SECURITY.md has a '{label}' row"));
    row.split(|c: char| !(c.is_ascii_digit() || c == '-'))
        .filter_map(|w| w.parse::<jiff::civil::Date>().ok())
        .next_back()
        .unwrap_or_else(|| panic!("the '{label}' row has a date: {row}"))
}

#[test]
fn the_policy_has_every_part_a_reporter_and_a_deployer_need() {
    let p = policy();
    for heading in [
        "## 1. Scope",
        "## 2. How to report a vulnerability",
        "## 3. What the project commits to",
        "## 4. Severity",
        "## 5. Disclosure",
        "## 6. Good-faith research",
        "## 7. Supported versions",
        "## 8. How the project keeps the software secure",
        "## For deploying organisations",
    ] {
        assert!(p.contains(heading), "SECURITY.md lost '{heading}'");
    }
    assert!(p.contains("Report a\n   vulnerability") || p.contains("Report a vulnerability"));
    assert!(p.contains("**Never send real personal data.**"));
}

#[test]
fn the_policy_is_not_past_its_review_date() {
    let p = policy();
    let reviewed = date_in_row(&p, "Last reviewed");
    let next = date_in_row(&p, "Next review");
    let today = jiff::Timestamp::now()
        .to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .date();
    assert!(
        reviewed <= today,
        "the last review ({reviewed}) is in the future"
    );
    assert!(
        next > reviewed && patient_administration_system::time::day_count(reviewed, next) <= 366,
        "a review is due within a year of the last ({reviewed} to {next})"
    );
    assert!(
        today <= next,
        "SECURITY.md was due for review on {next}: review it, then move both dates"
    );
}

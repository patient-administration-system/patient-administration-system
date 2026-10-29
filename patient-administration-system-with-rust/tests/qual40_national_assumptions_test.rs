//! A ratchet on national and regional names in the server source (QUAL-40, NFR-26).
//!
//! `governance/national-assumptions-baseline.csv` records, per file under `src/`, how many times a token that
//! names one country's or region's scheme appears (`NHS`, `Wales`, `WIMD`, `SNOMED`, `HIPAA`, `Europe/London` and
//! the like). A file that gains one, or a new file that has one, fails here: a national form belongs in a
//! profile or adapter, and the baseline is changed on purpose, with the register
//! (`governance/national-assumptions.md`) updated in the same change. A file that loses one is welcome; lower
//! its number. This counts text, so it does not show a national form is absent, only that none was added.

use std::collections::BTreeMap;
use std::path::Path;

const TOKENS: [&str; 14] = [
    "NHS",
    "Wales",
    "Welsh",
    "WIMD",
    "SNOMED",
    "UK GDPR",
    "HIPAA",
    "Medicare",
    "Medicaid",
    "DTAC",
    "DCB01",
    "US Core",
    "UK Core",
    "Europe/London",
];

fn count(dir: &Path, root: &Path, out: &mut BTreeMap<String, usize>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            count(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            let n: usize = TOKENS.iter().map(|t| text.matches(t).count()).sum();
            if n > 0 {
                let rel = path.strip_prefix(root).unwrap();
                out.insert(rel.to_string_lossy().replace('\\', "/"), n);
            }
        }
    }
}

#[test]
fn no_file_gains_a_national_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut now = BTreeMap::new();
    count(&root.join("src"), root, &mut now);
    let baseline: BTreeMap<String, usize> =
        std::fs::read_to_string(root.join("../governance/national-assumptions-baseline.csv"))
            .unwrap()
            .lines()
            .skip(1)
            .filter_map(|l| {
                let (file, n) = l.rsplit_once(',')?;
                Some((file.to_string(), n.parse().ok()?))
            })
            .collect();
    let grew: Vec<String> = now
        .iter()
        .filter(|(file, n)| **n > baseline.get(*file).copied().unwrap_or(0))
        .map(|(file, n)| {
            format!(
                "{file}: {n} now, {} allowed",
                baseline.get(file).copied().unwrap_or(0)
            )
        })
        .collect();
    assert!(
        grew.is_empty(),
        "a national or regional name entered the core (NFR-26); make it a profile, or raise the baseline in \
         governance/national-assumptions-baseline.csv and update governance/national-assumptions.md:\n{}",
        grew.join("\n")
    );
}

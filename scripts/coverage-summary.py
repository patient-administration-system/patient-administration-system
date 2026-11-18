#!/usr/bin/env python3
"""Print line coverage per crate from an LCOV file (QUAL-17, written by `just rust-db-coverage`).

`cargo tarpaulin` writes `lcov.info`; this reads its `SF:` (source file), `LH:` (lines hit) and `LF:` (lines
found) records, groups the files by crate, and prints a Markdown table with the total, so the CI run page
shows the same summary on every run.

    python3 scripts/coverage-summary.py [lcov.info]
    python3 scripts/coverage-summary.py --self-test
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

# Longest prefix first: the domain crate sits inside the server crate's directory.
CRATES = [
    ("patient-administration-system-with-rust/domain/", "domain"),
    ("patient-administration-system-with-rust/migrations/", "migrations"),
    ("patient-administration-system-with-rust/load/", "load"),
    ("patient-administration-system-with-rust/", "server"),
    ("patient-administration-system-frontend/", "frontend"),
]


def crate_of(path: str) -> str | None:
    p = path.replace("\\", "/")
    for prefix, name in CRATES:
        i = p.find(prefix)
        if i >= 0:
            rest = p[i + len(prefix):]
            # Tests and benchmarks are the measuring stick, not what is measured.
            if rest.startswith(("tests/", "benches/")):
                return None
            return name
    return None


def parse(text: str) -> dict[str, tuple[int, int]]:
    """Lines (hit, found) per crate."""
    totals: dict[str, list[int]] = {}
    current = None
    for line in text.splitlines():
        if line.startswith("SF:"):
            current = crate_of(line[3:])
        elif current and line.startswith("LH:"):
            totals.setdefault(current, [0, 0])[0] += int(line[3:])
        elif current and line.startswith("LF:"):
            totals.setdefault(current, [0, 0])[1] += int(line[3:])
        elif line == "end_of_record":
            current = None
    return {k: (v[0], v[1]) for k, v in totals.items()}


def pct(hit: int, found: int) -> str:
    return f"{100.0 * hit / found:.1f}%" if found else "n/a"


def table(totals: dict[str, tuple[int, int]]) -> str:
    if not totals:
        return "No coverage records found.\n"
    order = [name for _, name in CRATES]
    rows = ["| Crate | Lines hit | Lines | Coverage |", "|---|---:|---:|---:|"]
    hit_all = found_all = 0
    for name in sorted(totals, key=lambda n: order.index(n) if n in order else len(order)):
        hit, found = totals[name]
        hit_all += hit
        found_all += found
        rows.append(f"| {name} | {hit} | {found} | {pct(hit, found)} |")
    rows.append(f"| **total** | {hit_all} | {found_all} | **{pct(hit_all, found_all)}** |")
    return "\n".join(rows) + "\n"


def self_test() -> None:
    sample = "\n".join([
        "SF:/w/patient-administration-system-with-rust/src/lib.rs", "LF:10", "LH:8", "end_of_record",
        "SF:/w/patient-administration-system-with-rust/domain/src/time.rs", "LF:4", "LH:4", "end_of_record",
        "SF:/w/patient-administration-system-with-rust/tests/a_test.rs", "LF:50", "LH:50", "end_of_record",
        "SF:/w/patient-administration-system-frontend/src/app.rs", "LF:6", "LH:3", "end_of_record",
    ])
    totals = parse(sample)
    assert totals == {"server": (8, 10), "domain": (4, 4), "frontend": (3, 6)}, totals
    out = table(totals)
    assert "| domain | 4 | 4 | 100.0% |" in out, out
    assert "| server | 8 | 10 | 80.0% |" in out, out
    assert "| **total** | 15 | 20 | **75.0%** |" in out, out
    assert out.index("| domain") < out.index("| server"), "domain is listed before the server, as CRATES orders"
    assert "No coverage records" in table({})
    with tempfile.TemporaryDirectory() as d:
        f = Path(d) / "lcov.info"
        f.write_text(sample)
        assert parse(f.read_text()) == totals
    print("coverage-summary self-test passed")


def main(argv: list[str]) -> int:
    if argv[1:] == ["--self-test"]:
        self_test()
        return 0
    path = Path(argv[1]) if len(argv) > 1 else Path("lcov.info")
    if not path.exists():
        sys.stderr.write(f"{path} not found: run `just rust-db-coverage` first\n")
        return 1
    sys.stdout.write(table(parse(path.read_text())))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

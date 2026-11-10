#!/usr/bin/env python3
"""Check that every test the hazard log names still exists (SAFE-5).

The log is `governance/clinical-safety.md`. Its `Tests` column names each
test as `path::function`, relative to the Rust crate, separated by `<br>`;
`none` means the hazard's controls have no test. The check fails when a
named file is missing or no longer defines the function, and lists the
hazards with no test so the number is visible.

    python3 scripts/check-hazard-tests.py          # check and report
    python3 scripts/check-hazard-tests.py --self-test
"""

import pathlib
import re
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOG = ROOT / "governance" / "clinical-safety.md"
CRATE = ROOT / "patient-administration-system-with-rust"
NAMED = re.compile(r"`([^`:]+)::([A-Za-z0-9_]+)`")


def rows(text):
    """Yield (hazard id, tests cell) for each hazard row of the log."""
    header = None
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if cells and cells[0] == "ID":
            header = cells
            continue
        if header and re.fullmatch(r"H-\d+", cells[0]):
            if len(cells) != len(header):
                raise ValueError(f"{cells[0]}: {len(cells)} cells, header has {len(header)}")
            yield cells[0], cells[header.index("Tests")]


def check(text, crate):
    """Return (problems, untested hazard ids)."""
    problems, untested = [], []
    for hazard, cell in rows(text):
        if cell == "none":
            untested.append(hazard)
            continue
        named = NAMED.findall(cell)
        if not named:
            problems.append(f"{hazard}: the Tests cell is neither `none` nor a list of path::function")
        for path, fn in named:
            source = crate / path
            if not source.is_file():
                problems.append(f"{hazard}: {path} does not exist")
            elif not re.search(rf"\bfn {re.escape(fn)}\s*[(<]", source.read_text()):
                problems.append(f"{hazard}: {path} defines no fn {fn}")
    return problems, untested


def main():
    try:
        problems, untested = check(LOG.read_text(), CRATE)
    except (ValueError, KeyError) as e:
        print(f"check-hazard-tests: {e}", file=sys.stderr)
        return 1
    for p in problems:
        print(f"check-hazard-tests: {p}", file=sys.stderr)
    print(f"hazards with no named test: {len(untested)} ({', '.join(untested) or 'none'})")
    return 1 if problems else 0


def self_test():
    with tempfile.TemporaryDirectory() as d:
        crate = pathlib.Path(d)
        (crate / "tests").mkdir()
        (crate / "tests" / "a_test.rs").write_text("#[test]\nfn kept() {}\nasync fn also_kept() {}\n")
        head = "| ID | Hazard | Tests | L |\n|---|---|---|---|\n"
        good = head + "| H-01 | x | `tests/a_test.rs::kept`<br>`tests/a_test.rs::also_kept` | |\n| H-02 | y | none | |\n"
        assert check(good, crate) == ([], ["H-02"]), check(good, crate)
        renamed = good.replace("::kept`", "::renamed`")
        assert check(renamed, crate)[0] == ["H-01: tests/a_test.rs defines no fn renamed"]
        gone = good.replace("a_test.rs::kept", "gone_test.rs::kept")
        assert check(gone, crate)[0] == ["H-01: tests/gone_test.rs does not exist"]
        prefix = good.replace("::kept`", "::kep`")
        assert check(prefix, crate)[0], "a prefix of a function name is not the function"
        blank = good.replace("| none |", "| |")
        assert check(blank, crate)[0], "an empty cell must say none"
        try:
            list(rows(head + "| H-03 | too few |\n"))
            raise AssertionError("a short row must be refused")
        except ValueError:
            pass
    print("check-hazard-tests self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else main())

#!/usr/bin/env python3
"""Keep the hazard log's tests and code honest (SAFE-5, SAFE-6).

The log is `governance/clinical-safety.md`. Its hazard table has a `Tests`
column naming each test as `path::function`, relative to the Rust crate,
separated by `<br>`; `none` means the hazard's controls have no test. A
second table, `| ID | Code |`, names the code each hazard rests on.

The check fails when a named test file is missing or no longer defines the
function, or a named code path does not exist, and lists the hazards with no
test. With `--changed`, it also lists the hazards whose code or tests the
given files touch, for the CSO to look at again; that part never fails.

    python3 scripts/check-hazard-tests.py
    python3 scripts/check-hazard-tests.py --changed $(git diff --name-only origin/main...)
    python3 scripts/check-hazard-tests.py --self-test
"""

import pathlib
import re
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOG = ROOT / "governance" / "clinical-safety.md"
CRATE_DIR = "patient-administration-system-with-rust"
CRATE = ROOT / CRATE_DIR
NAMED = re.compile(r"`([^`:]+)::([A-Za-z0-9_]+)`")
PATH = re.compile(r"`([^`]+)`")


def rows(text, column):
    """Yield (hazard id, cell) for each hazard row of every table with `column`."""
    header = None
    for line in text.splitlines():
        if not line.startswith("|"):
            header = None
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if cells and cells[0] == "ID":
            header = cells if column in cells else None
            continue
        if header and re.fullmatch(r"H-\d+", cells[0]):
            if len(cells) != len(header):
                raise ValueError(f"{cells[0]}: {len(cells)} cells, header has {len(header)}")
            yield cells[0], cells[header.index(column)]


def tests_of(text):
    return {h: NAMED.findall(cell) for h, cell in rows(text, "Tests")}


def code_of(text):
    return {h: PATH.findall(cell) for h, cell in rows(text, "Code")}


def check(text, crate):
    """Return (problems, untested hazard ids)."""
    problems, untested = [], []
    for hazard, cell in rows(text, "Tests"):
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
    for hazard, paths in code_of(text).items():
        for path in paths:
            if not (crate / path).exists():
                problems.append(f"{hazard}: code path {path} does not exist")
    return problems, untested


def touched(text, changed):
    """The hazards whose code or named tests any of `changed` falls under."""
    files = set()
    for f in changed:
        f = f.strip()
        if f.startswith(CRATE_DIR + "/"):
            files.add(f[len(CRATE_DIR) + 1 :])
    tests, code = tests_of(text), code_of(text)
    hits = {}
    for hazard in list(tests) + [h for h in code if h not in tests]:
        why = sorted(
            {f for f in files for p in code.get(hazard, []) if f == p or (p.endswith("/") and f.startswith(p))}
            | {f for f in files for p, _ in tests.get(hazard, []) if f == p}
        )
        if why:
            hits[hazard] = why
    return hits


def main(argv):
    text = LOG.read_text()
    try:
        problems, untested = check(text, CRATE)
    except ValueError as e:
        print(f"check-hazard-tests: {e}", file=sys.stderr)
        return 1
    for p in problems:
        print(f"check-hazard-tests: {p}", file=sys.stderr)
    print(f"hazards with no named test: {len(untested)} ({', '.join(untested) or 'none'})")
    if "--changed" in argv:
        hits = touched(text, argv[argv.index("--changed") + 1 :])
        if hits:
            print("hazards whose code or tests this change touches (for the CSO to review):")
            for hazard, why in hits.items():
                print(f"  {hazard}: {', '.join(why)}")
        else:
            print("this change touches no hazard's listed code or tests")
    return 1 if problems else 0


def self_test():
    with tempfile.TemporaryDirectory() as d:
        crate = pathlib.Path(d)
        (crate / "tests").mkdir()
        (crate / "src" / "noshow").mkdir(parents=True)
        (crate / "tests" / "a_test.rs").write_text("#[test]\nfn kept() {}\nasync fn also_kept() {}\n")
        head = "| ID | Hazard | Tests | L |\n|---|---|---|---|\n"
        good = head + "| H-01 | x | `tests/a_test.rs::kept`<br>`tests/a_test.rs::also_kept` | |\n| H-02 | y | none | |\n"
        code = "\n| ID | Code |\n|---|---|\n| H-01 | `src/noshow/` |\n| H-02 | `src/noshow/` |\n"
        assert check(good + code, crate) == ([], ["H-02"]), check(good + code, crate)
        renamed = good.replace("::kept`", "::renamed`")
        assert check(renamed, crate)[0] == ["H-01: tests/a_test.rs defines no fn renamed"]
        gone = good.replace("a_test.rs::kept", "gone_test.rs::kept")
        assert check(gone, crate)[0] == ["H-01: tests/gone_test.rs does not exist"]
        prefix = good.replace("::kept`", "::kep`")
        assert check(prefix, crate)[0], "a prefix of a function name is not the function"
        blank = good.replace("| none |", "| |")
        assert check(blank, crate)[0], "an empty cell must say none"
        moved = good + code.replace("src/noshow/", "src/gone/", 1)
        assert check(moved, crate)[0] == ["H-01: code path src/gone/ does not exist"]
        try:
            list(rows(head + "| H-03 | too few |\n", "Tests"))
            raise AssertionError("a short row must be refused")
        except ValueError:
            pass
        # SAFE-6: a change under a hazard's code, or to its test, names it.
        p = CRATE_DIR + "/"
        hits = touched(good + code, [p + "src/noshow/model.rs", "README.md"])
        assert hits == {"H-01": ["src/noshow/model.rs"], "H-02": ["src/noshow/model.rs"]}, hits
        assert touched(good + code, [p + "tests/a_test.rs"]) == {"H-01": ["tests/a_test.rs"]}
        assert touched(good + code, [p + "src/noshowing.rs", p + "src/other/x.rs"]) == {}
    # The acceptance case against the real log: a change to src/noshow/ names H-10 and H-13.
    hits = touched(LOG.read_text(), [CRATE_DIR + "/src/noshow/model.rs"])
    assert {"H-10", "H-13"} <= set(hits), hits
    print("check-hazard-tests self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else main(sys.argv[1:]))

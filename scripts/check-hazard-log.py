#!/usr/bin/env python3
"""Keep the hazard log honest (SAFE-2, SAFE-3, SAFE-5, SAFE-6).

The log is `governance/clinical-safety.md`. Its hazard table has a `Tests`
column naming each test as `path::function`, relative to the Rust crate,
separated by `<br>`; `none` means the hazard's controls have no test. A
second table, `| ID | Code |`, names the code each hazard rests on.

The check fails when a named test file is missing or no longer defines the
function, or a named code path does not exist, and lists the hazards with no
test. With `--changed`, it also lists the hazards whose code or tests the
given files touch, for the CSO to look at again; that part never fails.

Closures (`governance/hazard-closures.csv`, append-only): each line closes,
accepts or reopens one hazard. A closure must be by the CSO or deputy named
in the log, on evidence submitted by someone else, with at least one item of
evidence whose reference cannot change (`commit=<40 hex>@date`,
`test=<path>::<fn>#run=<CI run>@date`, `document=<path>#sha256=<hex>@date`),
and the log's Status column must agree with the latest record. With
`--base REF`, a hazard row present at REF must still be there, and the
closure file must only have grown.

    python3 scripts/check-hazard-log.py
    python3 scripts/check-hazard-log.py --changed $(git diff --name-only origin/main...)
    python3 scripts/check-hazard-log.py --base HEAD^1
    python3 scripts/check-hazard-log.py --self-test
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


# --- Closures (SAFE-2, SAFE-3) -------------------------------------------------

CLOSURES = ROOT / "governance" / "hazard-closures.csv"
LOG_PATH = "governance/clinical-safety.md"
CLOSURES_PATH = "governance/hazard-closures.csv"
FIELDS = [
    "hazard", "reason", "residual_likelihood", "residual_severity", "evidence",
    "closed_by", "closed_by_role", "submitted_by", "at", "log_version", "note",
]
# The status each reason leaves a hazard in; a hazard with no record is open.
STATUS_AFTER = {
    "eliminated": "closed",
    "controlled": "closed",
    "not_applicable": "closed",
    "accepted": "accepted",
    "reopened": "open",
}
STATUSES = {"open", "accepted", "closed"}
EVIDENCE = {
    "commit": re.compile(r"[0-9a-f]{40}"),
    "test": re.compile(r"[^\s#]+::[A-Za-z0-9_]+#run=\d+"),
    "document": re.compile(r"[^\s#]+#sha256=[0-9a-f]{64}"),
}
ITEM = re.compile(r"(\w+)=(\S+)@(\d{4}-\d\d-\d\d)")
WHEN = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d(:\d\d)?(Z|[+-]\d\d:\d\d)")


def officers(text):
    """The named CSO and deputy; `not named` names nobody."""
    names = []
    for role in ("Clinical Safety Officer", "Deputy Clinical Safety Officer"):
        m = re.search(rf"^{role}: (.+?)\s*$", text, re.M)
        if not m:
            raise ValueError(f"the log has no '{role}:' line")
        name = re.sub(r"\s*\(.*\)$", "", m.group(1).rstrip(".")).strip()
        if name and not name.lower().startswith("not named"):
            names.append(name)
    return names


def check_closures(text, csv_text):
    """Return problems with the closure records and the log's Status column."""
    import csv
    import io

    problems = []
    hazards = {h for h, _ in rows(text, "Status")}
    status = dict(rows(text, "Status"))
    allowed = officers(text)
    reader = csv.DictReader(io.StringIO(csv_text))
    if reader.fieldnames != FIELDS:
        return [f"{CLOSURES_PATH}: the header must be {','.join(FIELDS)}"]
    latest = {}
    for n, r in enumerate(reader, start=2):
        where = f"{CLOSURES_PATH} line {n} ({r['hazard']})"
        if r["hazard"] not in hazards:
            problems.append(f"{where}: no such hazard in the log")
            continue
        reason = r["reason"]
        if reason not in STATUS_AFTER:
            problems.append(f"{where}: unknown reason {reason!r}")
            continue
        if not WHEN.fullmatch(r["at"]):
            problems.append(f"{where}: 'at' must be a date and time with its offset")
        if reason == "reopened":
            if not r["closed_by"] or not r["note"]:
                problems.append(f"{where}: a reopening needs who and why (closed_by, note)")
            latest[r["hazard"]] = reason
            continue
        if r["closed_by"] not in allowed:
            problems.append(
                f"{where}: closed_by must be the named CSO or deputy"
                + ("; none is named, so nothing can be closed" if not allowed else "")
            )
        if not r["submitted_by"] or r["submitted_by"] == r["closed_by"]:
            problems.append(f"{where}: the evidence must be submitted by a different person")
        if reason in ("controlled", "accepted") and not (r["residual_likelihood"] and r["residual_severity"]):
            problems.append(f"{where}: {reason} needs the residual likelihood and severity, set by a clinician")
        if not re.fullmatch(r"[0-9a-f]{7,40}", r["log_version"]):
            problems.append(f"{where}: log_version must be the commit of the log as it stood")
        items = [i for i in r["evidence"].split(";") if i.strip()]
        if not items:
            problems.append(f"{where}: a closure with no evidence is invalid")
        for item in items:
            m = ITEM.fullmatch(item.strip())
            if not m or m.group(1) not in EVIDENCE or not EVIDENCE[m.group(1)].fullmatch(m.group(2)):
                problems.append(f"{where}: evidence {item.strip()!r} is not kind=reference@date with a reference that cannot change")
        latest[r["hazard"]] = reason
    for hazard, st in status.items():
        if st not in STATUSES:
            problems.append(f"{hazard}: status {st!r} is not one of {', '.join(sorted(STATUSES))}")
            continue
        want = STATUS_AFTER.get(latest.get(hazard, "reopened"))
        if st != want:
            problems.append(f"{hazard}: the log says {st}, but its closure records leave it {want}")
    return problems


def at_base(base, path):
    """The file as it stood at git ref `base`, or None if it did not exist."""
    import subprocess

    r = subprocess.run(["git", "-C", str(ROOT), "show", f"{base}:{path}"], capture_output=True, text=True)
    if r.returncode != 0:
        if "does not exist" in r.stderr or "exists on disk, but not in" in r.stderr:
            return None
        raise ValueError(f"cannot read {path} at {base}: {r.stderr.strip()}")
    return r.stdout


def check_against_base(text, csv_text, base_text, base_csv):
    """No hazard row disappears; the closure file only grows."""
    problems = []
    if base_text is not None:
        before = set(re.findall(r"^\| (H-\d+) \|", base_text, re.M))
        now = set(re.findall(r"^\| (H-\d+) \|", text, re.M))
        for hazard in sorted(before - now):
            problems.append(f"{hazard} is in the log at the base but not now: a hazard is closed, never deleted")
    if base_csv is not None and not csv_text.startswith(base_csv):
        problems.append(f"{CLOSURES_PATH} changed a line it already had: it is append-only, a correction is a new line")
    return problems


def main(argv):
    text = LOG.read_text()
    try:
        problems, untested = check(text, CRATE)
        csv_text = CLOSURES.read_text()
        problems += check_closures(text, csv_text)
        if "--base" in argv:
            base = argv[argv.index("--base") + 1]
            problems += check_against_base(
                text, csv_text, at_base(base, LOG_PATH), at_base(base, CLOSURES_PATH)
            )
    except (ValueError, IndexError) as e:
        print(f"check-hazard-log: {e}", file=sys.stderr)
        return 1
    for p in problems:
        print(f"check-hazard-log: {p}", file=sys.stderr)
    print(f"hazards with no named test: {len(untested)} ({', '.join(untested) or 'none'})")
    if "--base" not in argv:
        print("rows and closures were not compared with a base (no --base): CI compares them")
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
    # SAFE-2 and SAFE-3: closures.
    log = (
        "Clinical Safety Officer: Dr A Clinician\nDeputy Clinical Safety Officer: not named (SAFE-1).\n\n"
        "| ID | Hazard | Status |\n|---|---|---|\n| H-01 | x | open |\n| H-02 | y | open |\n"
    )
    header = ",".join(FIELDS) + "\n"
    ev = "test=tests/a_test.rs::kept#run=42@2026-11-10;commit=" + "a" * 40 + "@2026-11-10"
    good_close = header + f'H-01,controlled,2,3,{ev},Dr A Clinician,CSO,B Developer,2026-11-10T12:00:00+00:00,abcdef1,\n'
    closed_log = log.replace("| H-01 | x | open |", "| H-01 | x | closed |")
    assert check_closures(closed_log, good_close) == [], check_closures(closed_log, good_close)
    assert check_closures(log, header) == []
    # A row closed with no record.
    assert check_closures(closed_log, header) == ["H-01: the log says closed, but its closure records leave it open"]
    # A record with no evidence is refused.
    bare = good_close.replace(ev, "")
    assert "H-01): a closure with no evidence is invalid" in " ".join(check_closures(closed_log, bare))
    # Evidence that can change (a branch name, a link) is refused.
    moving = good_close.replace("commit=" + "a" * 40, "commit=main")
    assert any("cannot change" in p for p in check_closures(closed_log, moving))
    # Only the named CSO or deputy; and with nobody named, nobody.
    by_dev = good_close.replace("Dr A Clinician,CSO", "B Developer,CSO")
    assert any("closed_by must be" in p for p in check_closures(closed_log, by_dev))
    unnamed = closed_log.replace("Dr A Clinician", "not named (SAFE-1).")
    assert any("none is named" in p for p in check_closures(unnamed, good_close))
    # The CSO cannot submit their own evidence.
    alone = good_close.replace("B Developer", "Dr A Clinician")
    assert any("different person" in p for p in check_closures(closed_log, alone))
    # Residual risk is set, never defaulted.
    unscored = good_close.replace("controlled,2,3", "controlled,,")
    assert any("residual" in p for p in check_closures(closed_log, unscored))
    # A reopening returns the hazard to open.
    reopened = good_close + "H-01,reopened,,,,B Developer,,,2026-11-11T09:00:00+00:00,,src/noshow changed\n"
    assert check_closures(log, reopened) == []
    assert check_closures(closed_log, reopened)
    # Against a base: a deleted row, and an edited closure line.
    gone = log.replace("| H-02 | y | open |\n", "")
    assert check_against_base(gone, header, log, header) == [
        "H-02 is in the log at the base but not now: a hazard is closed, never deleted"
    ]
    assert check_against_base(log, good_close + "x\n", log, good_close) == []
    edited = good_close.replace("2,3", "1,1")
    assert check_against_base(log, edited, log, good_close), "an edited closure line must be refused"
    assert check_against_base(log, header, None, None) == []
    assert officers(LOG.read_text()) == [], "the real log names no CSO yet (SAFE-1)"
    # The real log and closure file pass.
    assert check_closures(LOG.read_text(), CLOSURES.read_text()) == [], check_closures(LOG.read_text(), CLOSURES.read_text())
    # The acceptance case against the real log: a change to src/noshow/ names H-10 and H-13.
    hits = touched(LOG.read_text(), [CRATE_DIR + "/src/noshow/model.rs"])
    assert {"H-10", "H-13"} <= set(hits), hits
    print("check-hazard-log self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else main(sys.argv[1:]))

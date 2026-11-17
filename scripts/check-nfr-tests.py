#!/usr/bin/env python3
"""Every NFR has a test that names it (HUB-9).

An NFR in `spec/07-non-functional-requirements.md` passes when there is evidence of a test for it:

1. a test or source file names it (`NFR-12`, not `NFR-120`), under the Rust crate's `tests/`, `src/` and
   `load/`, the UI's `tests/`, or the operator front-end's `tests/`; or
2. its row's last column names a test file that exists (`tests/x_test.rs`, `ui/tests/...`, `*.spec.ts`,
   `*.test.ts`), as most rows do.

An NFR that no test can check, because the compiler or a CI job enforces it, is listed in ALLOWED below with
the reason. An NFR in ALLOWED that has gained evidence fails too, so the list cannot go stale.

`--self-test` checks the checker: removing the only citation of an NFR fails it.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "spec" / "07-non-functional-requirements.md"
RUST = ROOT / "patient-administration-system-with-rust"
UI = ROOT / "patient-administration-system-ui-with-svelte"
FRONTEND = ROOT / "patient-administration-system-frontend"
SEARCH = [RUST / "tests", RUST / "src", RUST / "load", UI / "tests", FRONTEND / "tests"]
SUFFIXES = {".rs", ".ts", ".js", ".mjs", ".py", ".sh"}

# NFRs a test cannot check, and what checks them instead.
ALLOWED = {
    "NFR-8": "the compiler: #![cfg_attr(not(test), forbid(unsafe_code))] in lib.rs",
    "NFR-9": "the CI job `deny` (cargo deny check)",
    "NFR-17": "NOT YET BUILT, stated in its row: there is one unversioned /api, so no header to test yet",
}


def rows(spec_text):
    """(id, last cell) of each NFR row."""
    out = []
    for line in spec_text.splitlines():
        m = re.match(r"^\| (NFR-\d+) \|(.*)\|\s*$", line)
        if m:
            cells = [c.strip() for c in m.group(2).split(" | ")]
            out.append((m.group(1), cells[-1] if cells else ""))
    return out


def cited_test_files(cell):
    """Test files a row's last cell names, as paths that may exist."""
    paths = []
    for span in re.findall(r"`([^`]+)`", cell):
        span = span.split("::")[0].strip()
        if not re.search(r"(^|/)tests/|\.spec\.ts$|\.test\.ts$", span):
            continue
        if span.startswith("ui/"):
            paths.append(UI / span[3:])
        elif span.startswith("patient-administration-system"):
            paths.append(ROOT / span)
        else:
            paths.append(RUST / span)
    return paths


def named_in(texts, nfr):
    pattern = re.compile(re.escape(nfr) + r"(?!\d)")
    return any(pattern.search(t) for t in texts)


def check(spec_rows, texts, exists, allowed):
    problems = []
    for nfr, cell in spec_rows:
        evidence = named_in(texts, nfr) or any(exists(p) for p in cited_test_files(cell))
        if nfr in allowed:
            if evidence:
                problems.append(f"{nfr} is in ALLOWED but now has a test: remove it from ALLOWED")
        elif not evidence:
            problems.append(f"{nfr} has no test that names it, and its row names no test file that exists")
    for nfr in allowed:
        if nfr not in {n for n, _ in spec_rows}:
            problems.append(f"{nfr} is in ALLOWED but is not in the spec")
    return problems


def corpus():
    texts = []
    for base in SEARCH:
        if base.exists():
            for p in base.rglob("*"):
                if p.is_file() and p.suffix in SUFFIXES and "node_modules" not in p.parts:
                    texts.append(p.read_text(encoding="utf-8", errors="replace"))
    return texts


def self_test():
    spec = [("NFR-1", "`tests/real_test.rs`"), ("NFR-2", "**Not yet tested**"), ("NFR-12", "none")]
    exists = lambda p: p.name == "real_test.rs"
    assert check(spec, ["// NFR-2 is tested here", "NFR-12 covered"], exists, {}) == []
    # The only citation of NFR-2 removed: it fails.
    assert check(spec, ["NFR-12 covered"], exists, {}) == [
        "NFR-2 has no test that names it, and its row names no test file that exists"
    ]
    # NFR-120 is not NFR-12.
    assert len(check(spec, ["NFR-2", "NFR-120"], exists, {})) == 1
    # A cited file that does not exist is no evidence.
    assert len(check(spec, ["NFR-2", "NFR-12"], lambda p: False, {})) == 1
    # An allowed NFR with no evidence passes; one that gains a test must leave the list.
    assert check(spec, ["NFR-12"], exists, {"NFR-2": "why"}) == []
    assert len(check(spec, ["NFR-2", "NFR-12"], exists, {"NFR-2": "why"})) == 1
    # A stale entry for an NFR the spec no longer has fails.
    assert len(check(spec, ["NFR-2", "NFR-12"], exists, {"NFR-99": "why"})) == 1
    print("check-nfr-tests self-test: ok")


def main():
    if "--self-test" in sys.argv:
        self_test()
        return 0
    spec_rows = rows(SPEC.read_text(encoding="utf-8"))
    problems = check(spec_rows, corpus(), lambda p: p.exists(), ALLOWED)
    if problems:
        print("\n".join(problems))
        return 1
    print(f"every NFR has a test or a stated reason ({len(spec_rows)} NFRs, {len(ALLOWED)} allowed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

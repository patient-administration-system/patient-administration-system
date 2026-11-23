#!/usr/bin/env python3
"""Review greps from spec/rust-checked-arithmetic (RCA-ENF-5, RCA-ENF-7), over production code only.

* RCA-ENF-2: no `unwrap_or_default`. Any hit fails.
* RCA-ENF-5: no `saturating_*`, `wrapping_*` or `overflowing_*`, and no checked result that is unwrapped or
  defaulted. Any hit fails.
* RCA-ENF-7: a parse or lookup whose error is discarded and replaced. Each hit must carry
  `// RCA-IN-3: absent only` on its line or the line before, or be listed in `scripts/rca-input-baseline.txt`
  (a ratchet: the count per file may fall, never rise).

Production code is every `.rs` file outside a `tests` directory, up to its first `#[cfg(test)]`.
Run from the repository root: `python3 scripts/check-checked-arithmetic.py`; `--write-baseline` rewrites the ratchet.
"""
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "rca-input-baseline.txt"
SKIP_DIRS = {"target", "node_modules", ".git", "tests"}

ENF5_WORD = re.compile(r"\b(saturating|wrapping|overflowing)_\w+")
ENF5_CHECKED = re.compile(
    r"checked_\w+\([^;]*?\)\s*\.(unwrap|expect|unwrap_unchecked|unwrap_or_default)\b", re.S
)
# RCA-ENF-2: `unwrap_or_default` is banned; a declared fallback is `unwrap_or(<value>)` with an RCA-DECL-1 comment.
ENF2 = re.compile(r"\.unwrap_or_default\b")
ENF7 = [
    re.compile(r"\.parse(::<[^>]+>)?\(\)[^;]*?\.(ok\(\)|unwrap_or|unwrap_or_default|unwrap_or_else)\b", re.S),
    re.compile(r"(env::var|var_os|get|get_param|query)\([^;]*?\.ok\(\)[^;]*?\.(unwrap_or|unwrap_or_default|unwrap_or_else)\b", re.S),
]
ANNOTATION = "RCA-IN-3: absent only"


def production_files():
    for path in sorted(ROOT.rglob("*.rs")):
        rel = path.relative_to(ROOT)
        if SKIP_DIRS & set(rel.parts):
            continue
        text = path.read_text()
        cut = text.find("#[cfg(test)]")
        yield rel.as_posix(), text if cut < 0 else text[:cut]


def line_of(text, offset):
    return text.count("\n", 0, offset) + 1


def main():
    failures = []
    enf7 = Counter()
    for rel, text in production_files():
        lines = text.splitlines()
        for m in ENF5_WORD.finditer(text):
            failures.append(f"{rel}:{line_of(text, m.start())}: RCA-ENF-5: {m.group(0)}")
        for m in ENF2.finditer(text):
            failures.append(f"{rel}:{line_of(text, m.start())}: RCA-ENF-2: unwrap_or_default is banned; write the fallback")
        for m in ENF5_CHECKED.finditer(text):
            failures.append(f"{rel}:{line_of(text, m.start())}: RCA-ENF-5: a checked result is unwrapped or defaulted")
        seen = set()
        for pattern in ENF7:
            for m in pattern.finditer(text):
                n = line_of(text, m.start())
                if n in seen:
                    continue
                seen.add(n)
                nearby = " ".join(lines[max(0, n - 2) : n])
                if ANNOTATION not in nearby:
                    enf7[rel] += 1
    if "--write-baseline" in sys.argv:
        BASELINE.write_text("".join(f"{n} {rel}\n" for rel, n in sorted(enf7.items())))
        print(f"wrote {len(enf7)} files, {sum(enf7.values())} hits")
        return 0
    allowed = {}
    if BASELINE.exists():
        for row in BASELINE.read_text().splitlines():
            n, rel = row.split(" ", 1)
            allowed[rel] = int(n)
    for rel, n in sorted(enf7.items()):
        if n > allowed.get(rel, 0):
            failures.append(
                f"{rel}: RCA-ENF-7: {n} parse or lookup errors discarded and replaced "
                f"(baseline {allowed.get(rel, 0)}); separate absent from invalid (RCA-IN-3)"
            )
    for rel, n in allowed.items():
        if enf7.get(rel, 0) < n:
            failures.append(f"{rel}: RCA-ENF-7: fewer hits than the baseline ({enf7.get(rel, 0)} < {n}); lower it with --write-baseline")
    for f in failures:
        print(f)
    print(f"{len(failures)} problem(s); {sum(enf7.values())} ENF-7 hit(s) held by the baseline")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

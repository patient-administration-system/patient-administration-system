#!/usr/bin/env python3
"""Print a Markdown table of Criterion results (PERF-3).

Reads every `<group>/<bench>/new/estimates.json` (and `benchmark.json` for the throughput) under a Criterion
directory, default `target/criterion`, and prints one row per benchmark: the median with its 95% interval and,
when the benchmark declared one, the throughput. The weekly Performance job writes it to the run page, so a
week's numbers can be read without downloading the reports.

    python3 scripts/criterion-summary.py [target/criterion]
    python3 scripts/criterion-summary.py --self-test
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path


def human_time(ns: float) -> str:
    for unit, size in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= size:
            return f"{ns / size:.3g} {unit}"
    return f"{ns:.3g} ns"


def human_rate(per_second: float, unit: str) -> str:
    for prefix, size in (("G", 1e9), ("M", 1e6), ("k", 1e3)):
        if per_second >= size:
            return f"{per_second / size:.3g} {prefix}{unit}/s"
    return f"{per_second:.3g} {unit}/s"


def rows(root: Path) -> list[tuple[str, str, str, str]]:
    out = []
    for estimates in sorted(root.glob("**/new/estimates.json")):
        bench_dir = estimates.parent.parent
        meta_path = estimates.parent / "benchmark.json"
        meta = json.loads(meta_path.read_text()) if meta_path.exists() else {}
        name = meta.get("full_id") or str(bench_dir.relative_to(root))
        median = json.loads(estimates.read_text())["median"]
        point = median["point_estimate"]
        low = median["confidence_interval"]["lower_bound"]
        high = median["confidence_interval"]["upper_bound"]
        rate = ""
        throughput = meta.get("throughput") or {}
        if point > 0:
            if "Bytes" in throughput:
                rate = human_rate(throughput["Bytes"] / (point / 1e9), "B")
            elif "Elements" in throughput:
                rate = human_rate(throughput["Elements"] / (point / 1e9), "elem")
        out.append((name, human_time(point), f"{human_time(low)} to {human_time(high)}", rate))
    return out


def table(root: Path) -> str:
    found = rows(root)
    if not found:
        return f"No Criterion results under `{root}`.\n"
    lines = ["| Benchmark | Median | 95% interval | Throughput |", "|---|---:|---|---:|"]
    lines += [f"| `{n}` | {m} | {i} | {r} |" for n, m, i, r in found]
    return "\n".join(lines) + "\n"


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        assert "No Criterion results" in table(root)
        new = root / "hub_redact" / "segments" / "100" / "new"
        new.mkdir(parents=True)
        (new / "estimates.json").write_text(json.dumps({"median": {
            "point_estimate": 12_500.0,
            "confidence_interval": {"lower_bound": 12_000.0, "upper_bound": 13_000.0},
        }}))
        (new / "benchmark.json").write_text(json.dumps({
            "full_id": "hub_redact/segments/100", "throughput": {"Bytes": 10_000},
        }))
        out = table(root)
        assert "| `hub_redact/segments/100` | 12.5 µs | 12 µs to 13 µs | 800 MB/s |" in out, out
        # A benchmark with no throughput and no benchmark.json is named by its directory.
        other = root / "backoff" / "new"
        other.mkdir(parents=True)
        (other / "estimates.json").write_text(json.dumps({"median": {
            "point_estimate": 3.2,
            "confidence_interval": {"lower_bound": 3.1, "upper_bound": 3.3},
        }}))
        assert "| `backoff` | 3.2 ns | 3.1 ns to 3.3 ns |  |" in table(root)
    print("criterion-summary self-test passed")


def main(argv: list[str]) -> int:
    if argv[1:] == ["--self-test"]:
        self_test()
        return 0
    root = Path(argv[1]) if len(argv) > 1 else Path("target/criterion")
    sys.stdout.write(table(root))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

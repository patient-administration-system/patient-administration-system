#!/usr/bin/env python3
"""Release gate (audit PA-08): publish only a commit whose own CI passed.

The release workflow runs this before any job that can write to a registry. It reads the `ci-ok` check run
(the aggregate job in ci.yml) for the exact commit being released and passes only when the latest one, made by
GitHub Actions for that commit, finished with success. A green result for another commit, a pending, missing,
cancelled, skipped or failed one all refuse. Reading needs only `checks: read`.

    python3 scripts/release-gate.py --sha SHA --checks check-runs.json     # decide from a saved API answer
    python3 scripts/release-gate.py --sha SHA --repo OWNER/NAME --wait 3600  # ask GitHub (gh, GH_TOKEN), waiting
                                                                             # while CI is still running
    python3 scripts/release-gate.py --self-test                             # fixtures, and the workflow graph

Exit 0: green. Exit 1: refused, with the reason on stderr. Exit 2: usage.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import time
from pathlib import Path

CHECK = "ci-ok"
APP = "github-actions"
POLL_SECONDS = 30


def decide(sha: str, answer: dict) -> tuple[str, str]:
    """('green'|'pending'|'refused', reason) for the check runs GitHub returned for `sha`."""
    runs = [
        r
        for r in answer.get("check_runs", [])
        if r.get("name") == CHECK
        and r.get("head_sha") == sha
        and (r.get("app") or {}).get("slug") == APP
    ]
    if not runs:
        others = [r.get("head_sha", "?")[:12] for r in answer.get("check_runs", []) if r.get("name") == CHECK]
        extra = f" (only for {', '.join(others)})" if others else ""
        return "refused", f"no {CHECK} check run from GitHub Actions for {sha[:12]}{extra}"
    latest = max(runs, key=lambda r: int(r.get("id", 0)))
    if latest.get("status") != "completed":
        return "pending", f"{CHECK} for {sha[:12]} is {latest.get('status')}"
    if latest.get("conclusion") != "success":
        return "refused", f"{CHECK} for {sha[:12]} concluded {latest.get('conclusion')}"
    return "green", f"{CHECK} passed for {sha[:12]}"


def ask(repo: str, sha: str) -> dict:
    out = subprocess.run(
        ["gh", "api", f"repos/{repo}/commits/{sha}/check-runs?check_name={CHECK}&filter=latest&per_page=100"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def gate(sha: str, fetch, wait: int, sleep=time.sleep, now=time.monotonic) -> int:
    deadline = now() + wait
    while True:
        verdict, reason = decide(sha, fetch())
        if verdict == "green":
            print(reason)
            return 0
        if verdict == "pending" and now() < deadline:
            print(f"{reason}; waiting", file=sys.stderr)
            sleep(POLL_SECONDS)
            continue
        print(f"release refused: {reason}", file=sys.stderr)
        return 1


# --- the workflow graph -------------------------------------------------------------------------------------

WRITES = ("packages: write", "contents: write", "attestations: write", "id-token: write")


def jobs_of(workflow: str) -> dict[str, dict]:
    """Each job's needs and whether it can write, read from the workflow text (no YAML library in CI)."""
    jobs: dict[str, dict] = {}
    body = workflow.split("\njobs:\n", 1)[1]
    for block in re.split(r"\n(?=  [A-Za-z0-9_-]+:\n)", "\n" + body):
        m = re.match(r"\n?  ([A-Za-z0-9_-]+):\n", block)
        if not m:
            continue
        needs = re.search(r"^    needs: \[([^\]]*)\]", block, re.M)
        jobs[m.group(1)] = {
            "needs": [n.strip() for n in needs.group(1).split(",")] if needs else [],
            "writes": any(w in block for w in WRITES),
            "gated": "release-gate.py" in block,
        }
    return jobs


def upstream(jobs: dict[str, dict], name: str) -> set[str]:
    seen: set[str] = set()
    todo = list(jobs[name]["needs"])
    while todo:
        n = todo.pop()
        if n not in seen:
            seen.add(n)
            todo.extend(jobs.get(n, {}).get("needs", []))
    return seen


def graph_problems(workflow: str) -> list[str]:
    jobs = jobs_of(workflow)
    gates = {n for n, j in jobs.items() if j["gated"]}
    problems = []
    if not gates:
        problems.append("no job runs release-gate.py")
    for name, job in jobs.items():
        if not job["writes"]:
            continue
        up = upstream(jobs, name)
        if not gates & up:
            problems.append(f"{name} can write but does not wait for the release gate")
        if "sbom" in jobs and "sbom" not in up:
            problems.append(f"{name} can write but does not wait for the SBOMs")
    for name in gates:
        if jobs[name]["writes"]:
            problems.append(f"the gate job {name} must only read")
    return problems


# --- self-test ---------------------------------------------------------------------------------------------

def self_test() -> int:
    sha, other = "a" * 40, "b" * 40

    def run(i, status="completed", conclusion="success", head=sha, name=CHECK, app=APP):
        return {"id": i, "name": name, "head_sha": head, "status": status, "conclusion": conclusion,
                "app": {"slug": app}}

    cases = [
        ("green for this commit", [run(1)], "green"),
        ("green for another commit only", [run(1, head=other)], "refused"),
        ("pending", [run(1, status="in_progress", conclusion=None)], "pending"),
        ("queued", [run(1, status="queued", conclusion=None)], "pending"),
        ("missing", [], "refused"),
        ("cancelled", [run(1, conclusion="cancelled")], "refused"),
        ("failed", [run(1, conclusion="failure")], "refused"),
        ("skipped", [run(1, conclusion="skipped")], "refused"),
        ("a green run of another check", [run(1, name="rust")], "refused"),
        ("a green ci-ok from another app", [run(1, app="someone-else")], "refused"),
        ("failed, then re-run green", [run(1, conclusion="failure"), run(2)], "green"),
        ("green, then re-run failed", [run(2, conclusion="failure"), run(1)], "refused"),
    ]
    failures = 0
    for what, runs, want in cases:
        got, reason = decide(sha, {"check_runs": runs})
        ok = got == want
        failures += not ok
        print(f"{'ok  ' if ok else 'FAIL'}  {what}: {got} ({reason})")

    # Waiting: pending twice then green passes; pending past the deadline refuses; nothing is slept after.
    answers = iter([[run(1, status="in_progress", conclusion=None)]] * 2 + [[run(1)]])
    clock = [0.0]
    sleep = lambda s: clock.__setitem__(0, clock[0] + s)
    got = gate(sha, lambda: {"check_runs": next(answers)}, 3600, sleep, lambda: clock[0])
    failures += got != 0
    print(f"{'ok  ' if got == 0 else 'FAIL'}  a pending run that turns green within the wait passes")
    clock = [0.0]
    got = gate(sha, lambda: {"check_runs": [run(1, status="queued", conclusion=None)]}, 60, sleep, lambda: clock[0])
    failures += got != 1
    print(f"{'ok  ' if got == 1 else 'FAIL'}  a run still pending at the deadline is refused")

    # The graph: the real workflow passes; one where images skip the gate or the SBOMs fails.
    real = (Path(__file__).resolve().parent.parent / ".github/workflows/release.yml").read_text()
    for what, text, want_ok in [
        ("release.yml: every job that can write waits for the gate and the SBOMs", real, True),
        ("a publisher that skips the gate is caught",
         real.replace("needs: [verify, deny, sbom, gate]", "needs: [verify, deny, sbom]"), False),
        ("a publisher that skips the SBOMs is caught",
         real.replace("needs: [verify, deny, sbom, gate]", "needs: [verify, deny, gate]"), False),
    ]:
        problems = graph_problems(text)
        ok = (not problems) == want_ok and text != (None if want_ok else real)
        failures += not ok
        print(f"{'ok  ' if ok else 'FAIL'}  {what}{'' if ok else ': ' + '; '.join(problems)}")

    if failures:
        print(f"{failures} failed")
        return 1
    print("all passed")
    return 0


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    args = dict(zip(argv[::2], argv[1::2]))
    if len(argv) % 2 or "--sha" not in args or not ({"--checks", "--repo"} & args.keys()):
        print(__doc__, file=sys.stderr)
        return 2
    sha = args["--sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        print(f"not a commit id: {sha}", file=sys.stderr)
        return 2
    if "--checks" in args:
        fetch = lambda: json.loads(Path(args["--checks"]).read_text())
        wait = 0
    else:
        fetch = lambda: ask(args["--repo"], sha)
        wait = int(args.get("--wait", "0"))
    return gate(sha, fetch, wait)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

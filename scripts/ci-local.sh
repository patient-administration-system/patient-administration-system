#!/usr/bin/env bash
# The local gate (QUAL-42): the `just` recipes of CI, run on one commit, isolated from the working tree.
#
#   scripts/ci-local.sh                      HEAD, all jobs
#   scripts/ci-local.sh <commit>             that commit
#   scripts/ci-local.sh --jobs "rust docs"   a subset
#   scripts/ci-local.sh --check-clock        only the clock check
#
# It uses a fresh `git worktree`, its own build directory (so no lock is shared with another session or an editor),
# and a database created for the run and dropped after it. It refuses to start when the database clock and this
# machine's differ by more than CI_LOCAL_MAX_SKEW seconds: a virtual machine that slept keeps its old time, and on
# 2026-11-04 a 74-minute drift failed two time-based tests for a reason that had nothing to do with the code.
#
# The record is target/ci-local/<sha>.json: jobs, durations, tool versions and architecture. It is a record, not an
# authority: this machine is not CI's (arm64 macOS against x86_64 Linux), and `ci-ok` stays the independent check
# (spec/11-testing-strategy.md, QUAL-43).
#
# Settings: PGHOST (127.0.0.1), PGPORT (5442), PGUSER (pas), PGPASSWORD (pas_dev_password), CI_LOCAL_MAX_SKEW (30).
# CI_LOCAL_DB_CLOCK_CMD replaces the query that reads the database clock; scripts/ci-local-test.sh uses it to
# simulate a skewed clock.
set -euo pipefail

export PGHOST="${PGHOST:-127.0.0.1}" PGPORT="${PGPORT:-5442}" PGUSER="${PGUSER:-pas}"
export PGPASSWORD="${PGPASSWORD:-pas_dev_password}"
max_skew="${CI_LOCAL_MAX_SKEW:-30}"

jobs="rust rust-db deny ui docs"
commit=HEAD
clock_only=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --jobs) jobs="$2"; shift 2 ;;
    --check-clock) clock_only=1; shift ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) commit="$1"; shift ;;
  esac
done

database_epoch() {
  if [[ -n "${CI_LOCAL_DB_CLOCK_CMD:-}" ]]; then
    bash -c "$CI_LOCAL_DB_CLOCK_CMD"
  else
    psql -d postgres -Atc 'select floor(extract(epoch from now()))::bigint'
  fi
}

check_clock() {
  local db host skew
  db="$(database_epoch)" || { echo "could not read the database clock on $PGHOST:$PGPORT" >&2; exit 1; }
  host="$(date +%s)"
  skew=$(( db - host ))
  if (( ${skew#-} > max_skew )); then
    echo "refusing to run: the database clock is ${skew}s from this machine's (limit ${max_skew}s)." >&2
    echo "Restart the database's virtual machine, then run again." >&2
    exit 3
  fi
  echo "clock: database and host differ by ${skew}s"
}

check_clock
if (( clock_only )); then exit 0; fi

root="$(git rev-parse --show-toplevel)"
sha="$(git rev-parse --verify "$commit^{commit}")"
out="$root/target/ci-local"
mkdir -p "$out"
worktree="$(mktemp -d -t pas-ci-local.XXXXXX)"
database="pas_ci_${sha:0:12}_$$"

cleanup() {
  git -C "$root" worktree remove --force "$worktree" 2>/dev/null || rm -rf "$worktree"
  git -C "$root" worktree prune
  psql -d postgres -qc "DROP DATABASE IF EXISTS \"$database\"" 2>/dev/null || true
}
trap cleanup EXIT

git -C "$root" worktree add --quiet --detach "$worktree" "$sha"
psql -d postgres -qc "CREATE DATABASE \"$database\""
export CARGO_TARGET_DIR="$out/build"
export DATABASE_URL="postgres://$PGUSER:$PGPASSWORD@$PGHOST:$PGPORT/$database"

results="$(mktemp -t pas-ci-local-results.XXXXXX)"
trap 'cleanup; rm -f "$results"' EXIT
failed=0
started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
for job in $jobs; do
  log="$out/$job.log"
  echo "--- $job"
  from="$(date +%s)"
  if (cd "$worktree" && just "$job") >"$log" 2>&1; then status=passed; else status=failed; failed=1; fi
  took=$(( $(date +%s) - from ))
  printf '%s\t%s\t%s\n' "$job" "$status" "$took" >>"$results"
  echo "    $status in ${took}s"
  if [[ "$status" == failed ]]; then tail -20 "$log" >&2; fi
done

python3 - "$out/$sha.json" "$results" "$sha" "$started" "$(uname -sm)" \
  "$(rustc --version)" "$(cargo --version)" "$(node --version)" "$(pnpm --version)" "$(just --version)" <<'PY'
import json, sys
path, results, sha, started, machine, rustc, cargo, node, pnpm, just = sys.argv[1:]
jobs = []
for line in open(results):
    name, status, seconds = line.rstrip("\n").split("\t")
    jobs.append({"job": name, "status": status, "seconds": int(seconds)})
record = {
    "commit": sha,
    "started": started,
    "machine": machine,
    "tools": {"rustc": rustc, "cargo": cargo, "node": node, "pnpm": pnpm, "just": just},
    "jobs": jobs,
    "total_seconds": sum(j["seconds"] for j in jobs),
    "passed": all(j["status"] == "passed" for j in jobs),
}
json.dump(record, open(path, "w"), indent=2)
print(f"record: {path}")
PY
exit "$failed"

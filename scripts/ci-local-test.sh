#!/usr/bin/env bash
# Tests of the local gate's clock check and of the quick hook's wiring (QUAL-42, QUAL-45). No database needed: the
# database clock is simulated, and the checks stop before anything is built.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
failures=0
expect() { # expect <description> <wanted exit> <command...>
  local what="$1" want="$2" got; shift 2
  "$@" >/dev/null 2>&1; got=$?
  if [[ "$got" == "$want" ]]; then echo "ok    $what"; else echo "FAIL  $what: exit $got, wanted $want"; failures=$((failures + 1)); fi
}
clock() { CI_LOCAL_DB_CLOCK_CMD="$1" "$here/ci-local.sh" --check-clock; }

expect "a clock 74 minutes slow is refused" 3 clock 'echo $(( $(date +%s) - 4440 ))'
expect "a clock 74 minutes fast is refused" 3 clock 'echo $(( $(date +%s) + 4440 ))'
expect "a clock 31 seconds slow is refused" 3 clock 'echo $(( $(date +%s) - 31 ))'
expect "a clock 5 seconds slow passes" 0 clock 'echo $(( $(date +%s) - 5 ))'
expect "a clock that cannot be read stops the run" 1 clock 'exit 1'
expect "an unknown option is refused" 2 "$here/ci-local.sh" --nonsense
expect "the hook is executable" 0 test -x "$here/../.githooks/pre-push"
expect "the hook runs the quick recipe" 0 grep -q '^exec just quick$' "$here/../.githooks/pre-push"

if (( failures )); then echo "$failures failed"; exit 1; fi
echo "all passed"

#!/usr/bin/env bash
# Infrastructure chaos: while a probe hits the API, kill the database (and optionally the identity provider)
# for a few seconds and check that the API (1) answers 503/5xx cleanly while it is down, never hangs, and (2) serves
# normally again once the database is back, without being restarted.
#
# DESTRUCTIVE to whatever runs in the named containers. Use a throwaway stack
# (PAS_PG_PORT=5543 podman compose -p chaos -f podman/compose.yml up -d), never a shared one.
#
#   CHAOS_I_UNDERSTAND=yes PAS_API=http://localhost:8080 CHAOS_PG=chaos-postgres-1 scripts/chaos-podman.sh
set -euo pipefail
[ "${CHAOS_I_UNDERSTAND:-}" = yes ] || { echo "Set CHAOS_I_UNDERSTAND=yes (this stops containers)."; exit 2; }
API="${PAS_API:-http://localhost:8080}"
PG="${CHAOS_PG:?name the database container, e.g. chaos-postgres-1}"
DOWN_SECONDS="${CHAOS_DOWN_SECONDS:-8}"
code() { curl -s -o /dev/null -m 5 -w '%{http_code}' "$API/api/patients?limit=1" || echo 000; }

[ "$(code)" = 200 ] || { echo "the API is not healthy before the test: $(code)"; exit 1; }
echo "stopping $PG for ${DOWN_SECONDS}s"
podman stop -t 1 "$PG" >/dev/null
end=$((SECONDS + DOWN_SECONDS)); worst=0; hung=0
while [ $SECONDS -lt $end ]; do
  start=$SECONDS; c=$(code); took=$((SECONDS - start))
  [ "$took" -gt 4 ] && hung=$((hung + 1))
  case "$c" in 200) echo "unexpected 200 while the database is down";; 5??|000) ;; *) echo "odd status $c";; esac
  sleep 1
done
podman start "$PG" >/dev/null
echo "restarted; waiting for the API to recover"
for i in $(seq 1 60); do
  [ "$(code)" = 200 ] && { echo "recovered after ${i}s without restarting the API"; [ "$hung" = 0 ] || { echo "FAIL: $hung probes hung"; exit 1; }; exit 0; }
  sleep 1
done
echo "FAIL: the API did not recover within 60s"; exit 1

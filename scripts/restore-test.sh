#!/usr/bin/env bash
# Restore drill: back up the live database, restore it into a throwaway database, and check that
# every table has the same row count. Exits non-zero on any difference. Leaves nothing behind.
#
#   PG_CONTAINER=podman-postgres-1 scripts/restore-test.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
db="${PGDATABASE:-pas}"
user="${PGUSER:-pas}"
scratch="${db}_restore_$$"
dump="$(mktemp -t pas-restore.XXXXXX)"
trap 'rm -f "$dump"; psql_admin "DROP DATABASE IF EXISTS \"$scratch\"" >/dev/null 2>&1 || true' EXIT

psql_admin() { # run SQL against the maintenance database
  if [[ -n "${PG_CONTAINER:-}" ]]; then podman exec "$PG_CONTAINER" psql -U "$user" -d postgres -tA -c "$1"
  else psql -tA -d postgres -c "$1"; fi
}
psql_in() { # run SQL against a named database
  if [[ -n "${PG_CONTAINER:-}" ]]; then podman exec "$PG_CONTAINER" psql -U "$user" -d "$1" -tA -c "$2"
  else psql -tA -d "$1" -c "$2"; fi
}
counts() { # "table count" lines, sorted
  local d="$1" t
  for t in $(psql_in "$d" "select tablename from pg_tables where schemaname='public' order by 1"); do
    echo "$t $(psql_in "$d" "select count(*) from \"$t\"")"
  done
}

"$here/backup.sh" "$dump" >/dev/null
psql_admin "CREATE DATABASE \"$scratch\"" >/dev/null
if [[ -n "${PG_CONTAINER:-}" ]]; then
  podman exec -i "$PG_CONTAINER" pg_restore -U "$user" -d "$scratch" --no-owner < "$dump"
else
  pg_restore -d "$scratch" --no-owner < "$dump"
fi

a="$(counts "$db")"; b="$(counts "$scratch")"
if [[ "$a" == "$b" ]]; then
  echo "restore drill passed: $(echo "$a" | wc -l | tr -d ' ') tables, row counts identical"
else
  echo "restore drill FAILED: row counts differ" >&2
  diff <(echo "$a") <(echo "$b") >&2 || true
  exit 1
fi

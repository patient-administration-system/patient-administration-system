#!/usr/bin/env bash
# Restore drill: back up the live database, restore it into a throwaway database, and check that
# every table has the same rows: a row count and an MD5 over the table's rows in a fixed order. Exits
# non-zero on any difference. Leaves nothing behind. Also checks the dump was written mode 0600.
#
#   PG_CONTAINER=podman-postgres-1 scripts/restore-test.sh
#
# RESTORE_DRILL_TAMPER=1 changes one restored row before comparing, to prove the drill notices (CI runs
# it both ways and expects this one to fail). Needs at least one non-empty table.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
db="${PGDATABASE:-pas}"
user="${PGUSER:-pas}"
scratch="${db}_restore_$$"
# A fresh directory, so backup.sh creates the dump itself and its umask decides the mode.
dir="$(mktemp -d -t pas-restore.XXXXXX)"
dump="$dir/pas.dump"
trap 'rm -rf "$dir"; psql_admin "DROP DATABASE IF EXISTS \"$scratch\"" >/dev/null 2>&1 || true' EXIT

psql_admin() { # run SQL against the maintenance database
  if [[ -n "${PG_CONTAINER:-}" ]]; then podman exec "$PG_CONTAINER" psql -U "$user" -d postgres -tA -c "$1"
  else psql -tA -d postgres -c "$1"; fi
}
psql_in() { # run SQL against a named database
  if [[ -n "${PG_CONTAINER:-}" ]]; then podman exec "$PG_CONTAINER" psql -U "$user" -d "$1" -tA -c "$2"
  else psql -tA -d "$1" -c "$2"; fi
}
contents() { # "table count md5" lines, sorted by table
  local d="$1" t
  for t in $(psql_in "$d" "select tablename from pg_tables where schemaname='public' order by 1"); do
    echo "$t $(psql_in "$d" "select count(*) || ' ' || coalesce(md5(string_agg(r::text, E'\\n' order by r::text)), '-') from \"$t\" r")"
  done
}

"$here/backup.sh" "$dump" >/dev/null
mode="$(stat -c %a "$dump" 2>/dev/null || stat -f %Lp "$dump")"
[[ "$mode" == 600 ]] || { echo "restore drill FAILED: the dump is mode $mode, not 600" >&2; exit 1; }
psql_admin "CREATE DATABASE \"$scratch\"" >/dev/null
if [[ -n "${PG_CONTAINER:-}" ]]; then
  podman exec -i "$PG_CONTAINER" pg_restore -U "$user" -d "$scratch" --no-owner < "$dump"
else
  pg_restore -d "$scratch" --no-owner < "$dump"
fi

if [[ -n "${RESTORE_DRILL_TAMPER:-}" ]]; then
  victim=""
  for t in $(psql_in "$scratch" "select tablename from pg_tables where schemaname='public' order by 1"); do
    [[ "$(psql_in "$scratch" "select exists (select 1 from \"$t\")")" == t ]] && { victim="$t"; break; }
  done
  [[ -n "$victim" ]] || { echo "nothing to tamper with: every table is empty" >&2; exit 2; }
  # Blank one value in a nullable column if any row has one; otherwise remove a row.
  changed=0
  for col in $(psql_in "$scratch" "select column_name from information_schema.columns where table_schema='public' and table_name='$victim' and is_nullable='YES' order by ordinal_position"); do
    changed="$(psql_in "$scratch" "with u as (update \"$victim\" set \"$col\" = NULL where ctid = (select ctid from \"$victim\" where \"$col\" is not null limit 1) returning 1) select count(*) from u")"
    [[ "$changed" == 1 ]] && break
  done
  [[ "$changed" == 1 ]] || psql_in "$scratch" "delete from \"$victim\" where ctid = (select ctid from \"$victim\" limit 1)" >/dev/null
  echo "tampered with one row of $victim in the restored copy" >&2
fi

a="$(contents "$db")"; b="$(contents "$scratch")"
total="$(echo "$a" | awk '{n += $2} END {print n + 0}')"
if [[ "$a" == "$b" ]]; then
  echo "restore drill passed: $(echo "$a" | wc -l | tr -d ' ') tables, $total rows, contents identical"
else
  echo "restore drill FAILED: contents differ" >&2
  diff <(echo "$a") <(echo "$b") >&2 || true
  exit 1
fi

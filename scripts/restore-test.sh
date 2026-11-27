#!/usr/bin/env bash
# Restore drill: back up the live database, restore it into a throwaway database, and check that every
# table has the same rows. Leaves nothing behind. Also checks the dump was written mode 0600.
#
#   PG_CONTAINER=podman-postgres-1 scripts/restore-test.sh
#
# The comparison is with the backup's own moment, not with the live database afterwards (audit PA-12): a
# transaction exports a snapshot, the dump is taken from that snapshot, and the source's checksums are read
# from it too, so writes during the drill cannot fail a good restore. Each table's checksum is its row count
# and the sum of a hash of each row, which needs constant memory however large the table.
#
# Exit status: 0 the restore is identical; 1 the contents differ; 2 the drill itself failed (a query, the
# dump or the restore), so a tool failure is never mistaken for a difference or the other way round.
#
# RESTORE_DRILL_TAMPER=1 changes one restored row before comparing, to prove the drill notices (CI runs
# it both ways and expects this one to exit 1). Needs at least one non-empty table.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
db="${PGDATABASE:-pas}"
user="${PGUSER:-pas}"
scratch="${db}_restore_$$"
# A fresh directory, so backup.sh creates the dump itself and its umask decides the mode.
dir="$(mktemp -d -t pas-restore.XXXXXX)"
dump="$dir/pas.dump"
holder=""
cleanup() {
  touch "$dir/stop" 2>/dev/null || true
  [[ -n "$holder" ]] && wait "$holder" 2>/dev/null || true
  rm -rf "$dir"
  psql_admin "DROP DATABASE IF EXISTS \"$scratch\"" >/dev/null 2>&1 || true
}
trap cleanup EXIT
fail() { echo "restore drill could not run: $*" >&2; exit 2; }

psql_cmd() { # psql against database $1 that stops at the first error; extra arguments follow
  local d="$1"; shift
  if [[ -n "${PG_CONTAINER:-}" ]]; then podman exec -i "$PG_CONTAINER" psql -X -U "$user" -d "$d" -v ON_ERROR_STOP=1 -q -tA "$@"
  else psql -X -d "$d" -v ON_ERROR_STOP=1 -q -tA "$@"; fi
}
psql_admin() { psql_cmd postgres -c "$1"; }
psql_in() { psql_cmd "$1" -c "$2"; }

# Every public table as "table count checksum", sorted, in one statement (so one error fails the lot).
manifest_sql="select t.tablename || ' ' || (xpath('/row/c/text()', x))[1]::text || ' ' || (xpath('/row/h/text()', x))[1]::text \
  from pg_tables t, query_to_xml(format('select count(*) as c, coalesce(sum((''x'' || substr(md5(r::text), 1, 15))::bit(60)::bigint)::text, ''-'') as h from %I.%I r', t.schemaname, t.tablename), false, true, '') x \
  where t.schemaname = 'public' order by t.tablename"

# Hold a transaction open that exports the snapshot the dump and the source checksums are taken from.
( printf 'BEGIN ISOLATION LEVEL REPEATABLE READ;\nSELECT pg_export_snapshot();\n'
  while [[ ! -e "$dir/stop" ]]; do sleep 0.2; done
  printf 'COMMIT;\n' ) | psql_cmd "$db" > "$dir/snapshot" 2> "$dir/snapshot.err" &
holder=$!
for _ in $(seq 1 100); do
  [[ -s "$dir/snapshot" ]] && break
  kill -0 "$holder" 2>/dev/null || fail "no snapshot: $(cat "$dir/snapshot.err")"
  sleep 0.1
done
snapshot="$(head -n 1 "$dir/snapshot")"
[[ -n "$snapshot" ]] || fail "no snapshot was exported"

PAS_BACKUP_SNAPSHOT="$snapshot" "$here/backup.sh" "$dump" >/dev/null || fail "the backup failed"
mode="$(stat -c %a "$dump" 2>/dev/null || stat -f %Lp "$dump")"
[[ "$mode" == 600 ]] || { echo "restore drill FAILED: the dump is mode $mode, not 600" >&2; exit 1; }
a="$(psql_cmd "$db" -c "BEGIN ISOLATION LEVEL REPEATABLE READ; SET TRANSACTION SNAPSHOT '$snapshot'; $manifest_sql; COMMIT;")" \
  || fail "the source checksums could not be read"
touch "$dir/stop"

psql_admin "CREATE DATABASE \"$scratch\"" >/dev/null || fail "the scratch database could not be made"
started="$(date +%s)"
if [[ -n "${PG_CONTAINER:-}" ]]; then
  podman exec -i "$PG_CONTAINER" sh -c 'pg_restore -U "$1" -d "$2" --no-owner; s=$?; cat > /dev/null; exit $s' sh "$user" "$scratch" < "$dump" \
    || fail "pg_restore failed"
else
  pg_restore -d "$scratch" --no-owner "$dump" || fail "pg_restore failed"
fi
restore_seconds=$(( $(date +%s) - started ))

if [[ -n "${RESTORE_DRILL_TAMPER:-}" ]]; then
  # Blank one value, so the row count stays the same and only the checksum can notice: the first nullable
  # column, in any table, that has a value. With none anywhere, remove a row of the first non-empty table.
  changed=""
  while IFS="|" read -r t col; do
    [[ -n "$t" ]] || continue
    n="$(psql_in "$scratch" "with u as (update \"$t\" set \"$col\" = NULL where ctid = (select ctid from \"$t\" where \"$col\" is not null limit 1) returning 1) select count(*) from u")"
    if [[ "$n" == 1 ]]; then changed="$t.$col"; break; fi
  done < <(psql_in "$scratch" "select table_name, column_name from information_schema.columns where table_schema='public' and is_nullable='YES' order by table_name, ordinal_position")
  if [[ -n "$changed" ]]; then
    echo "tampered with one value ($changed) in the restored copy" >&2
  else
    victim="$(psql_in "$scratch" "select tablename from pg_tables where schemaname='public' order by 1" | while read -r t; do
      [[ "$(psql_in "$scratch" "select exists (select 1 from \"$t\")")" == t ]] && { echo "$t"; break; }; done)"
    [[ -n "$victim" ]] || fail "nothing to tamper with: every table is empty"
    psql_in "$scratch" "delete from \"$victim\" where ctid = (select ctid from \"$victim\" limit 1)" >/dev/null
    echo "tampered with one row of $victim in the restored copy" >&2
  fi
fi

b="$(psql_in "$scratch" "$manifest_sql")" || fail "the restored checksums could not be read"
total="$(echo "$a" | awk '{n += $2} END {print n + 0}')"
if [[ "$a" == "$b" ]]; then
  echo "restore drill passed: $(echo "$a" | wc -l | tr -d ' ') tables, $total rows, contents identical; restored in ${restore_seconds}s"
else
  echo "restore drill FAILED: contents differ" >&2
  diff <(echo "$a") <(echo "$b") >&2 || true
  exit 1
fi

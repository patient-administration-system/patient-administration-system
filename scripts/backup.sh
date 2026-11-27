#!/usr/bin/env bash
# Back up the PAS database to a compressed custom-format dump.
#
#   scripts/backup.sh [output-file]
#
# Connection: PGHOST/PGPORT/PGUSER/PGPASSWORD/PGDATABASE as usual, or run inside the compose
# database container with PG_CONTAINER=podman-postgres-1. The dump contains personal data: store
# and transmit it as you would the database.
#
# The dump is written to a private temporary file beside the destination, checked (not empty, and an
# archive pg_restore can list), and only then renamed over the destination. A failed, empty or
# interrupted dump leaves an existing backup exactly as it was (PA-04).
set -euo pipefail
# The dump holds personal data: readable by its owner only (0600), whatever the caller's umask. A new
# file is created for every run, so an old destination's broader mode is not kept.
umask 077
out="${1:-pas-$(date -u +%Y%m%dT%H%M%SZ).dump}"
db="${PGDATABASE:-pas}"
user="${PGUSER:-pas}"
# PAS_BACKUP_SNAPSHOT: dump the state of an exported snapshot (the restore drill compares with it).
snapshot=()
[[ -n "${PAS_BACKUP_SNAPSHOT:-}" ]] && snapshot=(--snapshot="$PAS_BACKUP_SNAPSHOT")

dir="$(dirname -- "$out")"
tmp="$(mktemp "$dir/.$(basename -- "$out").XXXXXX")"
trap 'rm -f -- "$tmp"' EXIT

if [[ -n "${PG_CONTAINER:-}" ]]; then
  podman exec "$PG_CONTAINER" pg_dump -U "$user" -d "$db" --format=custom --no-owner ${snapshot[@]+"${snapshot[@]}"} > "$tmp"
  # pg_restore reads only the archive's table of contents; drain the rest so podman does not report a broken pipe.
  list() { podman exec -i "$PG_CONTAINER" sh -c 'pg_restore --list > /dev/null; s=$?; cat > /dev/null; exit $s' < "$1"; }
else
  pg_dump --format=custom --no-owner ${snapshot[@]+"${snapshot[@]}"} "$db" > "$tmp"
  list() { pg_restore --list "$1" > /dev/null; }
fi
[[ -s "$tmp" ]] || { echo "backup is empty; $out is unchanged" >&2; exit 1; }
list "$tmp" || { echo "backup is not a readable archive; $out is unchanged" >&2; exit 1; }
mv -f -- "$tmp" "$out"
trap - EXIT
echo "$out ($(wc -c < "$out" | tr -d ' ') bytes)"

#!/usr/bin/env bash
# Back up the PAS database to a compressed custom-format dump.
#
#   scripts/backup.sh [output-file]
#
# Connection: PGHOST/PGPORT/PGUSER/PGPASSWORD/PGDATABASE as usual, or run inside the compose
# database container with PG_CONTAINER=podman-postgres-1. The dump contains personal data: store
# and transmit it as you would the database.
set -euo pipefail
# The dump holds personal data: readable by its owner only (0600), whatever the caller's umask.
umask 077
out="${1:-pas-$(date -u +%Y%m%dT%H%M%SZ).dump}"
db="${PGDATABASE:-pas}"
user="${PGUSER:-pas}"

if [[ -n "${PG_CONTAINER:-}" ]]; then
  podman exec "$PG_CONTAINER" pg_dump -U "$user" -d "$db" --format=custom --no-owner > "$out"
else
  pg_dump --format=custom --no-owner "$db" > "$out"
fi
[[ -s "$out" ]] || { echo "backup is empty: $out" >&2; exit 1; }
echo "$out ($(wc -c < "$out" | tr -d ' ') bytes)"

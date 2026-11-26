#!/usr/bin/env bash
# Tests of scripts/backup.sh with stand-in pg_dump and pg_restore commands (PA-04): no database needed.
# A failed, empty or unreadable dump leaves an existing backup byte for byte; a good one replaces it with
# mode 0600 whatever the old file's mode was.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d -t pas-backup-test.XXXXXX)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
# pg_dump does what BACKUP_TEST_DUMP says; pg_restore --list accepts only a file that starts with PGDMP.
cat > "$work/bin/pg_dump" <<'EOF'
#!/usr/bin/env bash
case "$BACKUP_TEST_DUMP" in
  fail) printf 'PGDMP partial'; exit 1 ;;
  empty) exit 0 ;;
  garbage) printf 'not an archive' ;;
  good) printf 'PGDMP a new archive' ;;
esac
EOF
cat > "$work/bin/pg_restore" <<'EOF'
#!/usr/bin/env bash
[[ "$(head -c 5 "${@: -1}")" == PGDMP ]]
EOF
chmod +x "$work/bin/pg_dump" "$work/bin/pg_restore"

dest="$work/pas.dump"
mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"; }
failures=0
check() { # name, then a command that succeeds when the check holds
  local name="$1"
  shift
  if "$@"; then echo "ok: $name"; else echo "FAILED: $name" >&2; failures=$((failures + 1)); fi
}
holds() { [[ "$(cat "$dest")" == "$1" ]]; }
no_temporary_file() { [[ -z "$(find "$work" -maxdepth 1 -name '.pas.dump.*')" ]]; }
is_private() { [[ "$(mode "$dest")" == 600 ]]; }

for kind in fail empty garbage; do
  printf 'PGDMP the old archive' > "$dest"
  chmod 644 "$dest"
  if BACKUP_TEST_DUMP=$kind PATH="$work/bin:$PATH" "$here/backup.sh" "$dest" > /dev/null 2>&1; then
    check "a $kind dump is refused" false
  else
    check "a $kind dump is refused" true
  fi
  check "a $kind dump leaves the old backup" holds "PGDMP the old archive"
  check "a $kind dump leaves no temporary file" no_temporary_file
done

printf 'PGDMP the old archive' > "$dest"
chmod 644 "$dest"
BACKUP_TEST_DUMP=good PATH="$work/bin:$PATH" "$here/backup.sh" "$dest" > /dev/null
check "a good dump replaces the backup" holds "PGDMP a new archive"
check "a good dump is mode 600 though the old file was 644" is_private
check "a good dump leaves no temporary file" no_temporary_file

(( failures == 0 )) || { echo "backup-test: $failures failure(s)" >&2; exit 1; }
echo "backup-test: all passed"

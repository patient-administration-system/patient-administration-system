# Backup and restore

## Backup

```sh
PG_CONTAINER=podman-postgres-1 scripts/backup.sh pas.dump      # container
PGHOST=… PGUSER=pas PGPASSWORD=… scripts/backup.sh pas.dump    # remote
```

The dump is written to a private temporary file beside the destination and renamed over it only once it is non-empty and `pg_restore --list` can read it, so a failed, empty or interrupted dump leaves the previous backup as it was and the script exits non-zero. Every new dump is mode 0600, whatever the mode of the file it replaces. `scripts/backup-test.sh` (in `just script-tests`) checks this with stand-in commands.

The dump is PostgreSQL custom format. **It contains personal data**; encrypt it at rest and in transit, restrict access, and apply the retention schedule to backups too.

Back up Keycloak's database as well; without it nobody can sign in. The upstream `person-service` and `worker-service` have their own backups, and a PAS restore is only consistent with them if all are restored to a close point in time.

## Restore

```sh
createdb pas_new && pg_restore -d pas_new --no-owner pas.dump
```

Point `DATABASE_URL` at it, then run `pas-migrate up` (a no-op if the dump is current).

## Restore drill

```sh
PG_CONTAINER=podman-postgres-1 scripts/restore-test.sh
```

Backs up the live database, restores it into a throwaway one, compares every table, and drops the scratch database. The comparison is with the backup's own moment: a transaction exports a snapshot, `pg_dump --snapshot` dumps it, and the source's checksums are read from the same snapshot, so writes to the live database during the drill do not fail a good restore. Each table's checksum is its row count and the sum of a hash of each row (constant memory however large the table). It exits 1 when the contents differ, 2 when the drill itself could not run (a query, the dump or the restore failed), and 1 if the dump was not written mode 0600 (`backup.sh` sets `umask 077`: the dump holds personal data). It reports how long the restore took; compare it with your recovery time objective. `RESTORE_DRILL_TAMPER=1` blanks one restored value (the row count stays the same) to prove the comparison notices, and `just restore` requires that run to exit 1; CI runs the drill on seeded rows (`scripts/restore-seed.sql`) both ways. Run it after every schema change and on a schedule. It was run on 2026-10-18 against the development database (30 tables, identical counts); that is evidence the procedure works, not that any production backup exists.

## Not yet decided

Backup frequency, retention, off-site copy, recovery time and recovery point objectives. These belong to the deploying organisation.

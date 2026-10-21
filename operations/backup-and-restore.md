# Backup and restore

## Backup

```sh
PG_CONTAINER=podman-postgres-1 scripts/backup.sh pas.dump      # container
PGHOST=… PGUSER=pas PGPASSWORD=… scripts/backup.sh pas.dump    # remote
```

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

Backs up the live database, restores it into a throwaway one, compares every table's row count and an MD5 over its rows in a fixed order, and drops the scratch database. It exits non-zero on any difference, and also if the dump was not written mode 0600 (`backup.sh` sets `umask 077`: the dump holds personal data). `RESTORE_DRILL_TAMPER=1` alters one restored row first, to prove the comparison notices; CI runs the drill on seeded rows (`scripts/restore-seed.sql`) both ways. Run it after every schema change and on a schedule. It was run on 2026-10-18 against the development database (30 tables, identical counts); that is evidence the procedure works, not that any production backup exists.

## Not yet decided

Backup frequency, retention, off-site copy, recovery time and recovery point objectives. These belong to the deploying organisation.

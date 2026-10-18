# 10. Persistence

PostgreSQL 18. Schema is owned by the migration crate `patient-administration-system-migrations` (`patient-administration-system-with-rust/migrations`), shared by path with the front-end. Migrations are forward-only and applied with `pas-migrate up`.

- Identity tables were dropped in `m20260601_000006_delegate_identity`; `patient_id` columns became `person_id` and `practitioner_id` family columns became `worker_id`.
- `m20260602_000007_appointment_series_worker_id` finished that rename for `appointment_series`, which had been missed.
- A migration MUST be safe to run twice where it can be (guard renames with an existence check).
- Container volume: Postgres 18 images want the volume at `/var/lib/postgresql`, not `/var/lib/postgresql/data`.

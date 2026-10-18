//! Identity + workforce delegation (v0.44.0 Option C).
//!
//! Forward-only destructive migration that retires the four
//! identity/workforce tables and renames every dependent FK column:
//!
//! - `DROP TABLE patients, practitioners, practitioner_roles, departments`
//!   — identity + workforce moves to `person-service` /
//!   `worker-service` over HTTP. The PAS no longer holds these rows.
//! - `RENAME COLUMN patient_id TO person_id` on every dependent table
//!   (11 columns) — the value is now an opaque UUID referencing
//!   `person-service`, no DB-level FK across the service boundary.
//! - `RENAME COLUMN practitioner_id TO worker_id` on every dependent table
//!   (5 columns; includes the prefixed `admitting_/discharging_/referring_`
//!   variants).
//! - `CREATE TABLE upstream_inbox` to dedupe inbound merge webhooks from
//!   the two upstream services (`X-Upstream-Event-Id` primary key).
//!
//! This migration is **forward-only**: `down` is intentionally unimplemented
//! because there is no path to reconstruct the dropped patient/practitioner
//! rows once they are gone. Existing deployments must export to
//! `person-service` / `worker-service` out-of-band before
//! applying this migration. See `spec.md` §13 Layer 11C +
//! risk callout #2.
//!
//! CASCADE on the four DROP TABLE statements is intentional: it removes
//! every dependent FK constraint, partial-unique index, and partial-index
//! definition that referenced the dropped tables. Renamed columns keep
//! their data; only the column header changes.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // --- 1. Drop the four identity/workforce tables (CASCADE so every
        // FK referencing them goes away in one statement each).
        for ddl in [
            "DROP TABLE IF EXISTS patients CASCADE",
            "DROP TABLE IF EXISTS practitioners CASCADE",
            "DROP TABLE IF EXISTS practitioner_roles CASCADE",
            "DROP TABLE IF EXISTS departments CASCADE",
        ] {
            conn.execute_unprepared(ddl).await?;
        }

        // --- 2. Rename `patient_id` → `person_id` on 10 dependent tables
        // plus the v0.9 appointment_series table. CASCADE in step 1 already
        // removed the FK constraints, so the rename is a plain column
        // header change. Indexes carry through automatically in Postgres.
        for ddl in [
            "ALTER TABLE encounters RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE appointments RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE appointment_series RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE referrals RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE waitlist_entries RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE rtt_pathways RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE generated_letters RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE accounts RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE consents RENAME COLUMN patient_id TO person_id",
            "ALTER TABLE coverages RENAME COLUMN patient_id TO person_id",
        ] {
            conn.execute_unprepared(ddl).await?;
        }

        // --- 3. Rename `practitioner_id` family → `worker_id` family on the
        // five dependent tables. The bare-name variant on `encounters` and
        // `appointments`; the prefixed variants on `admissions`,
        // `discharges`, and `referrals`.
        for ddl in [
            "ALTER TABLE encounters RENAME COLUMN practitioner_id TO worker_id",
            "ALTER TABLE appointments RENAME COLUMN practitioner_id TO worker_id",
            "ALTER TABLE admissions RENAME COLUMN admitting_practitioner_id TO admitting_worker_id",
            "ALTER TABLE discharges RENAME COLUMN discharging_practitioner_id TO discharging_worker_id",
            "ALTER TABLE referrals RENAME COLUMN referring_practitioner_id TO referring_worker_id",
        ] {
            conn.execute_unprepared(ddl).await?;
        }

        // --- 4. Recreate the coverages unique partial index under the new
        // column name. The init+coverage migration created it as
        // `(patient_id, payor_name, policy_number) WHERE status = 'active'`.
        // RENAME COLUMN preserves the underlying B-tree but the index
        // *name* and its CREATE definition still reference `patient_id`
        // in pg_class metadata. Rather than introspect, drop and recreate
        // under a fresh name keyed to `person_id`.
        conn.execute_unprepared("DROP INDEX IF EXISTS ux_coverages_active_policy")
            .await?;
        conn.execute_unprepared(
            "CREATE UNIQUE INDEX ux_coverages_active_policy \
             ON coverages (person_id, payor_name, policy_number) \
             WHERE status = 'active'",
        )
        .await?;

        // --- 5. Upstream inbox: dedupe inbound merge webhooks from
        // `person-service` and `worker-service`. The receiver
        // looks up `event_id` before applying side-effects; replays are
        // no-ops.
        conn.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS upstream_inbox ( \
                event_id UUID PRIMARY KEY, \
                source TEXT NOT NULL, \
                received_at TIMESTAMPTZ NOT NULL DEFAULT NOW() \
            )",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_upstream_inbox_received_at \
             ON upstream_inbox (received_at)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Forward-only. There is no path to reconstruct the dropped
        // patient/practitioner rows. See spec.md §13 Layer 11C and risk
        // callout #2.
        Err(DbErr::Custom(
            "m20260601_000006_delegate_identity is forward-only — \
             reverting would lose all identity / workforce data. \
             Restore from backup if you need the prior schema."
                .to_string(),
        ))
    }
}

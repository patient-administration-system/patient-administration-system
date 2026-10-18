//! CAP-3: released slots and the standby list.
//!
//! `slot_releases` has one row each time a future slot is freed by a cancellation or a miss, and is
//! closed when anyone books that slot again: that is how the booking team sees whether a release
//! was refilled and how fast. `standby_entries` holds patients who have said they can come at short
//! notice, with the tags a coordinator needs to choose well (language, transport, phone only).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS slot_releases ( \
               id uuid PRIMARY KEY, \
               slot_id uuid NOT NULL, \
               appointment_id uuid NOT NULL, \
               service_type text NULL, \
               start_datetime timestamptz NOT NULL, \
               released_at timestamptz NOT NULL, \
               notice_days integer NULL CHECK (notice_days IS NULL OR notice_days >= 0), \
               release_class text NOT NULL CHECK (release_class IN ('early_release','late_release','silent_miss')), \
               status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','refilled')), \
               refilled_by_appointment_id uuid NULL, \
               refilled_at timestamptz NULL, \
               seconds_to_refill bigint NULL CHECK (seconds_to_refill IS NULL OR seconds_to_refill >= 0), \
               created_at timestamptz NOT NULL DEFAULT now())",
            // At most one open release per slot at a time.
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_slot_releases_open_slot ON slot_releases (slot_id) WHERE status = 'open'",
            "CREATE INDEX IF NOT EXISTS ix_slot_releases_released_at ON slot_releases (released_at)",
            "CREATE TABLE IF NOT EXISTS standby_entries ( \
               id uuid PRIMARY KEY, \
               person_id uuid NOT NULL, \
               service_type text NOT NULL, \
               short_notice_ok boolean NOT NULL DEFAULT true, \
               language text NULL, \
               needs_transport boolean NOT NULL DEFAULT false, \
               phone_only boolean NOT NULL DEFAULT false, \
               active boolean NOT NULL DEFAULT true, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               updated_at timestamptz NOT NULL DEFAULT now())",
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_standby_active ON standby_entries (person_id, service_type) WHERE active",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS standby_entries")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS slot_releases")
            .await?;
        Ok(())
    }
}

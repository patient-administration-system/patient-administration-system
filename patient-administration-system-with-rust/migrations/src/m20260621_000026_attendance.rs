//! CORE-4: when a patient arrived and left, and where they are now.
//!
//! `arrived_at` and `completed_at` are stamped by check-in and complete; `current_room` is a short label a
//! person sets when they move the patient (a move is audited). The in-facility list is everyone arrived and
//! not yet completed.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE appointments \
               ADD COLUMN IF NOT EXISTS arrived_at timestamptz NULL, \
               ADD COLUMN IF NOT EXISTS completed_at timestamptz NULL, \
               ADD COLUMN IF NOT EXISTS current_room text NULL",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_current_room_chk \
               CHECK (current_room IS NULL OR (length(btrim(current_room)) BETWEEN 1 AND 60))",
            "CREATE INDEX IF NOT EXISTS idx_appointments_in_facility ON appointments (arrived_at) WHERE status = 'arrived'",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

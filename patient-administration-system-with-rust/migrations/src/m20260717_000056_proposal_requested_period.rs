//! The window a `$find` asked for, kept on each proposal (NEW-10). The IHE Scheduling profile requires
//! `Appointment.requestedPeriod` on every appointment it returns; the window is what that period is.
//! Proposals made before this migration have none and answer with their own slot's time instead.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE slot_proposals \
                   ADD COLUMN IF NOT EXISTS requested_start timestamptz NULL, \
                   ADD COLUMN IF NOT EXISTS requested_end timestamptz NULL",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE slot_proposals DROP COLUMN IF EXISTS requested_end, \
                   DROP COLUMN IF EXISTS requested_start",
            )
            .await?;
        Ok(())
    }
}

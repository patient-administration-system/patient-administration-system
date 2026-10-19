//! A video visit that fails to connect falls back to a phone call (CAP-14): `staff_call_tasks` learns the
//! reason `video_failed`, so someone phones the patient instead of the visit being lost.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const REASONS: &str =
    "'silent','reschedule_request','urgent_pathway','transport_failed','high_risk'";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check",
        )
        .await?;
        db.execute_unprepared(&format!(
            "ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ({REASONS},'video_failed'))"
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DELETE FROM staff_call_tasks WHERE reason = 'video_failed'")
            .await?;
        db.execute_unprepared(
            "ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check",
        )
        .await?;
        db.execute_unprepared(&format!(
            "ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ({REASONS}))"
        ))
        .await?;
        Ok(())
    }
}

//! When a patient's communication needs were last reviewed, and by whom (NEW-20, the Accessible Information
//! Standard's sixth step). Both or neither.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE patient_communication_profiles \
               ADD COLUMN IF NOT EXISTS needs_reviewed_at timestamptz, \
               ADD COLUMN IF NOT EXISTS needs_reviewed_by text",
            "ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_needs_review_chk \
               CHECK ((needs_reviewed_at IS NULL AND needs_reviewed_by IS NULL) \
                   OR (needs_reviewed_at IS NOT NULL AND length(btrim(needs_reviewed_by)) > 0))",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_needs_review_chk",
            "ALTER TABLE patient_communication_profiles \
               DROP COLUMN IF EXISTS needs_reviewed_by, DROP COLUMN IF EXISTS needs_reviewed_at",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

//! A clinician's priority for a waiting-list entry and the date it must be looked at again (NEW-17). All
//! three or none. The priority is shown to staff; nothing orders, offers or books by it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE waitlist_entries \
               ADD COLUMN IF NOT EXISTS clinical_priority text, \
               ADD COLUMN IF NOT EXISTS clinical_priority_set_by text, \
               ADD COLUMN IF NOT EXISTS clinical_priority_review_by date",
            "ALTER TABLE waitlist_entries ADD CONSTRAINT waitlist_entries_clinical_priority_chk CHECK ( \
               (clinical_priority IS NULL AND clinical_priority_set_by IS NULL AND clinical_priority_review_by IS NULL) \
               OR (clinical_priority IN ('P1a','P1b','P2','P3','P4','P5','P6') \
                   AND length(btrim(clinical_priority_set_by)) > 0 AND clinical_priority_review_by IS NOT NULL))",
            "CREATE INDEX IF NOT EXISTS idx_waitlist_priority_review ON waitlist_entries (clinical_priority_review_by) \
               WHERE clinical_priority_review_by IS NOT NULL",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "DROP INDEX IF EXISTS idx_waitlist_priority_review",
            "ALTER TABLE waitlist_entries DROP CONSTRAINT IF EXISTS waitlist_entries_clinical_priority_chk",
            "ALTER TABLE waitlist_entries DROP COLUMN IF EXISTS clinical_priority_review_by, \
               DROP COLUMN IF EXISTS clinical_priority_set_by, DROP COLUMN IF EXISTS clinical_priority",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

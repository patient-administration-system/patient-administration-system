//! Follow-up target dates (NEW-15): the date by which a waiting-list entry's follow-up is due, and who set
//! it. Both or neither; the delayed-follow-up report counts entries past their target.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE waitlist_entries \
               ADD COLUMN IF NOT EXISTS follow_up_target date, \
               ADD COLUMN IF NOT EXISTS follow_up_target_set_by text",
            "ALTER TABLE waitlist_entries ADD CONSTRAINT waitlist_entries_follow_up_target_chk CHECK ( \
               (follow_up_target IS NULL AND follow_up_target_set_by IS NULL) \
               OR (follow_up_target IS NOT NULL AND length(btrim(follow_up_target_set_by)) > 0))",
            "CREATE INDEX IF NOT EXISTS idx_waitlist_follow_up_target ON waitlist_entries (follow_up_target) \
               WHERE follow_up_target IS NOT NULL",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "DROP INDEX IF EXISTS idx_waitlist_follow_up_target",
            "ALTER TABLE waitlist_entries DROP CONSTRAINT IF EXISTS waitlist_entries_follow_up_target_chk",
            "ALTER TABLE waitlist_entries \
               DROP COLUMN IF EXISTS follow_up_target_set_by, DROP COLUMN IF EXISTS follow_up_target",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

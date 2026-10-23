//! When a clinician first saw an emergency attendance (NEW-23). Arrival is `encounters.period_start`; this
//! adds the second end of the interval the triage target is measured over. Both columns are set together,
//! once, by a named person.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE encounters \
               ADD COLUMN IF NOT EXISTS first_contact_at timestamptz NULL, \
               ADD COLUMN IF NOT EXISTS first_contact_by text NULL",
            "ALTER TABLE encounters DROP CONSTRAINT IF EXISTS chk_encounters_first_contact",
            "ALTER TABLE encounters ADD CONSTRAINT chk_encounters_first_contact CHECK ( \
               (first_contact_at IS NULL AND first_contact_by IS NULL) \
               OR (first_contact_at IS NOT NULL AND first_contact_by IS NOT NULL \
                   AND length(btrim(first_contact_by)) > 0))",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE encounters DROP CONSTRAINT IF EXISTS chk_encounters_first_contact",
            "ALTER TABLE encounters DROP COLUMN IF EXISTS first_contact_by, DROP COLUMN IF EXISTS first_contact_at",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

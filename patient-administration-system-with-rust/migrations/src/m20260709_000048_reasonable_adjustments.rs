//! Reasonable adjustments as a coded record (NEW-6). One row per adjustment a named person recorded for a
//! patient, shaped like the NHS England Reasonable Adjustment Digital Flag: a category, a code, an optional
//! note, a review date, and when and by whom it was recorded and ended.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE IF NOT EXISTS reasonable_adjustments ( \
               id uuid PRIMARY KEY, \
               person_id uuid NOT NULL, \
               category text NOT NULL CHECK (category IN \
                 ('communication','physical_access','sensory_environment','understanding','appointment_handling')), \
               code text NOT NULL CHECK (length(btrim(code)) BETWEEN 1 AND 50), \
               note text CHECK (note IS NULL OR length(btrim(note)) BETWEEN 1 AND 200), \
               review_by date, \
               recorded_by text NOT NULL CHECK (length(btrim(recorded_by)) > 0), \
               recorded_at timestamptz NOT NULL, \
               ended_by text, \
               ended_at timestamptz, \
               CHECK ((ended_at IS NULL) = (ended_by IS NULL)))",
            // One live adjustment of a given code per patient; an ended one may be recorded again.
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_reasonable_adjustments_live ON reasonable_adjustments (person_id, code) \
               WHERE ended_at IS NULL",
            "CREATE INDEX IF NOT EXISTS idx_reasonable_adjustments_person ON reasonable_adjustments (person_id)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS reasonable_adjustments")
            .await?;
        Ok(())
    }
}

//! A triage category assigned elsewhere and recorded against an emergency encounter (NEW-22). One row per
//! assessment, so a re-triage keeps the history. The PAS records; it never works out a category.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE IF NOT EXISTS triage_assessments ( \
               id uuid PRIMARY KEY, \
               encounter_id uuid NOT NULL REFERENCES encounters (id), \
               triage_system text NOT NULL CHECK (triage_system IN ('mts')), \
               system_version text, \
               category text NOT NULL CHECK (category IN ('immediate','very_urgent','urgent','standard','non_urgent')), \
               flowchart text, \
               assessed_by text NOT NULL CHECK (length(btrim(assessed_by)) > 0), \
               assessed_at timestamptz NOT NULL, \
               created_at timestamptz NOT NULL DEFAULT now())",
            "CREATE INDEX IF NOT EXISTS idx_triage_assessments_encounter ON triage_assessments (encounter_id, assessed_at)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS triage_assessments")
            .await?;
        Ok(())
    }
}

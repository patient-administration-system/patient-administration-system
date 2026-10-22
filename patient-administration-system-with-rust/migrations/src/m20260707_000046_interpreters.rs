//! Interpreting needs and interpreter requests (NEW-5). The profile says what interpreting a patient needs
//! (a spoken language, British Sign Language, or deafblind interpreting); booking that patient raises one
//! interpreter request for the appointment, which a person books (WITS has no API) and confirms.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE patient_communication_profiles \
               ADD COLUMN IF NOT EXISTS interpreter_kind text, \
               ADD COLUMN IF NOT EXISTS interpreter_language text",
            // A spoken-language need names the language; BSL and deafblind interpreting do not take one.
            "ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_interpreter_chk \
               CHECK ((interpreter_kind IS NULL AND interpreter_language IS NULL) \
                   OR (interpreter_kind = 'spoken' AND length(btrim(interpreter_language)) BETWEEN 2 AND 35) \
                   OR (interpreter_kind IN ('bsl','deafblind') AND interpreter_language IS NULL))",
            "CREATE TABLE IF NOT EXISTS interpreter_requests ( \
               id uuid PRIMARY KEY, \
               appointment_id uuid NOT NULL UNIQUE, \
               person_id uuid NOT NULL, \
               kind text NOT NULL CHECK (kind IN ('spoken','bsl','deafblind')), \
               language text, \
               status text NOT NULL DEFAULT 'requested' CHECK (status IN ('requested','confirmed','cancelled')), \
               booking_ref text CHECK (booking_ref IS NULL OR length(btrim(booking_ref)) BETWEEN 1 AND 100), \
               confirmed_by text, \
               confirmed_at timestamptz, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               updated_at timestamptz NOT NULL DEFAULT now(), \
               CHECK ((status = 'confirmed') = (confirmed_at IS NOT NULL AND length(btrim(confirmed_by)) > 0)))",
            "CREATE INDEX IF NOT EXISTS idx_interpreter_requests_open ON interpreter_requests (status) \
               WHERE status = 'requested'",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "DROP TABLE IF EXISTS interpreter_requests",
            "ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_interpreter_chk",
            "ALTER TABLE patient_communication_profiles \
               DROP COLUMN IF EXISTS interpreter_language, DROP COLUMN IF EXISTS interpreter_kind",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

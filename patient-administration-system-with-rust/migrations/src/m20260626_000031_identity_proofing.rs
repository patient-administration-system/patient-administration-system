//! DTAC-9: identity proofing before a sign-in is linked to a patient record.
//!
//! A person who signs in through the identity provider is just an account until staff have checked who they
//! are and recorded it here: which account (`subject`, the provider's id for the person), which patient, how
//! it was checked, and who did. Until a live row exists the patient portal refuses the account, whatever
//! `person_id` the provider's attribute says. Revoking (a mistaken link, a bereavement, a safeguarding
//! concern) is a row update, never a delete, so the history stays.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS identity_proofings ( \
               id uuid PRIMARY KEY, \
               subject text NOT NULL CHECK (length(btrim(subject)) BETWEEN 1 AND 200), \
               person_id uuid NOT NULL, \
               method text NOT NULL CHECK (method IN ('in_person','video_id_check','nhs_login','postal_code')), \
               proofed_by text NOT NULL CHECK (length(btrim(proofed_by)) > 0), \
               proofed_at timestamptz NOT NULL DEFAULT now(), \
               revoked_at timestamptz NULL, \
               revoked_by text NULL, \
               CHECK ((revoked_at IS NULL AND revoked_by IS NULL) OR (revoked_at IS NOT NULL AND length(btrim(revoked_by)) > 0)))",
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_identity_proofings_live ON identity_proofings (subject) WHERE revoked_at IS NULL",
            "CREATE INDEX IF NOT EXISTS idx_identity_proofings_person ON identity_proofings (person_id)",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS identity_proofings")
            .await?;
        Ok(())
    }
}

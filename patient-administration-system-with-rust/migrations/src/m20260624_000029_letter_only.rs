//! CAS-10: a patient who can only be reached by letter.
//!
//! `letter_only` joins the accessibility needs a profile can record. Once set, no text, WhatsApp, email or
//! portal message is generated for the patient: the choke point every message passes through refuses it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_accessibility_needs_check",
            "ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_accessibility_needs_check \
               CHECK (accessibility_needs <@ ARRAY['easy_read','large_print','audio','british_sign_language','letter_only'])",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Fails while a profile still records `letter_only`: a down never rewrites data.
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_accessibility_needs_check",
            "ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_accessibility_needs_check \
               CHECK (accessibility_needs <@ ARRAY['easy_read','large_print','audio','british_sign_language'])",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }
}

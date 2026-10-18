//! CAP-4: reliable contact details and communication needs, owned by the PAS and keyed by
//! `person_id` (demographics stay in `person-service`). Every shape rule is also a CHECK so a bad
//! value cannot arrive by any route.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS patient_communication_profiles ( \
                   person_id uuid PRIMARY KEY, \
                   mobile_e164 text NULL CHECK (mobile_e164 IS NULL OR mobile_e164 ~ '^\\+[1-9][0-9]{7,14}$'), \
                   mobile_verified_at timestamptz NULL, \
                   mobile_verification_method text NULL CHECK (mobile_verification_method IS NULL OR mobile_verification_method IN ('staff','portal','sms_code')), \
                   alternate_name text NULL, \
                   alternate_relationship text NULL, \
                   alternate_phone_e164 text NULL CHECK (alternate_phone_e164 IS NULL OR alternate_phone_e164 ~ '^\\+[1-9][0-9]{7,14}$'), \
                   alternate_email text NULL, \
                   alternate_is_carer boolean NOT NULL DEFAULT false, \
                   alternate_is_emergency boolean NOT NULL DEFAULT false, \
                   preferred_channel text NULL CHECK (preferred_channel IS NULL OR preferred_channel IN ('print','email','sms','whatsapp','apple_messages','portal')), \
                   preferred_language text NULL, \
                   accessibility_needs text[] NOT NULL DEFAULT '{}' \
                     CHECK (accessibility_needs <@ ARRAY['easy_read','large_print','audio','british_sign_language']), \
                   updated_at timestamptz NOT NULL DEFAULT now(), \
                   updated_by text NULL, \
                   CHECK (mobile_verified_at IS NULL OR mobile_e164 IS NOT NULL))",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS patient_communication_profiles")
            .await?;
        Ok(())
    }
}

//! CAP-14: turn an appointment into a video or phone consultation.
//!
//! - `appointments.modality` (default `in_person`) and the conversion's own state, so a conversion is
//!   visible, reviewable and reversible. The barrier that led to it is kept separately from the
//!   cancellation barrier so measuring "why did people ask for a virtual visit" never mixes with "why
//!   did they cancel".
//! - `virtual_ok_services`: the services a clinician has approved for conversion, with who approved. A
//!   patient or the PAS never decides that a service can go virtual.
//! - `inbound_replies` learns the `convert` action and records the modality asked for.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE appointments \
               ADD COLUMN IF NOT EXISTS modality text NOT NULL DEFAULT 'in_person', \
               ADD COLUMN IF NOT EXISTS conversion_state text NULL, \
               ADD COLUMN IF NOT EXISTS conversion_barrier_code text NULL, \
               ADD COLUMN IF NOT EXISTS converted_at timestamptz NULL",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_modality_chk \
               CHECK (modality IN ('in_person','video','phone'))",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_conversion_state_chk \
               CHECK (conversion_state IS NULL OR conversion_state IN ('pending_review','confirmed','vetoed'))",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_conversion_barrier_chk \
               CHECK (conversion_barrier_code IS NULL OR conversion_barrier_code IN \
               ('cannot_travel','caring','work','feel_better','unwell','forgot','other','anxiety','cost','no_longer_needed','could_not_cancel','access_need'))",
            "CREATE TABLE IF NOT EXISTS virtual_ok_services ( \
               service_type text PRIMARY KEY, \
               approved_by text NOT NULL CHECK (length(btrim(approved_by)) > 0), \
               approved_at timestamptz NOT NULL DEFAULT now())",
            "ALTER TABLE inbound_replies ADD COLUMN IF NOT EXISTS modality text NULL",
            "ALTER TABLE inbound_replies DROP CONSTRAINT IF EXISTS inbound_replies_action_check",
            "ALTER TABLE inbound_replies ADD CONSTRAINT inbound_replies_action_check \
               CHECK (action IN ('confirm','cancel','rebook','convert'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Restoring a narrower CHECK fails, and the down stops, while a row still uses a newer value:
        // a down never deletes or rewrites data to make room.
        let db = manager.get_connection();
        for ddl in [
            "DROP TABLE IF EXISTS virtual_ok_services",
            "ALTER TABLE inbound_replies DROP CONSTRAINT IF EXISTS inbound_replies_action_check",
            "ALTER TABLE inbound_replies ADD CONSTRAINT inbound_replies_action_check CHECK (action IN ('confirm','cancel','rebook'))",
            "ALTER TABLE inbound_replies DROP COLUMN IF EXISTS modality",
            "ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_conversion_barrier_chk, \
               DROP CONSTRAINT IF EXISTS appointments_conversion_state_chk, DROP CONSTRAINT IF EXISTS appointments_modality_chk",
            "ALTER TABLE appointments DROP COLUMN IF EXISTS converted_at, DROP COLUMN IF EXISTS conversion_barrier_code, \
               DROP COLUMN IF EXISTS conversion_state, DROP COLUMN IF EXISTS modality",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }
}

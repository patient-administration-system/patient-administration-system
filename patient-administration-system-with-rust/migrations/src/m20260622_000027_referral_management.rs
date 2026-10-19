//! CORE-1: referral management.
//!
//! The `referrals` table existed with nothing using it. It now carries where the referral came from
//! (`gp`, `e-rs`, `internal`), the sender's own reference (an e-RS UBRN), and a status. Receiving the same
//! `(source, external_ref)` twice is one referral. `appointments.referral_id` links a booked appointment
//! back to the referral that caused it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE referrals \
               ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT 'internal', \
               ADD COLUMN IF NOT EXISTS external_ref text NULL, \
               ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'received', \
               ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now()",
            "ALTER TABLE referrals ADD CONSTRAINT referrals_source_chk CHECK (source IN ('gp','e-rs','internal'))",
            "ALTER TABLE referrals ADD CONSTRAINT referrals_status_chk CHECK (status IN ('received','waitlisted','booked','closed'))",
            "ALTER TABLE referrals ADD CONSTRAINT referrals_external_ref_chk \
               CHECK (external_ref IS NULL OR length(btrim(external_ref)) BETWEEN 1 AND 100)",
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_referrals_source_ref ON referrals (source, external_ref) WHERE external_ref IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_referrals_open ON referrals (received_at) WHERE status IN ('received','waitlisted')",
            "ALTER TABLE appointments ADD COLUMN IF NOT EXISTS referral_id uuid NULL",
            "CREATE INDEX IF NOT EXISTS idx_appointments_referral ON appointments (referral_id) WHERE referral_id IS NOT NULL",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

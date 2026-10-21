//! A consultant's triage of a referral (NEW-11): the referral can be returned to the GP with advice, or
//! redirected to another service, before anything is booked. Two new statuses, who triaged and when, the
//! service it went to and the new referral that carries the patient there (a redirect never loses the
//! patient). The advice itself is clinical and stays in the clinical record.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_status_chk",
            "ALTER TABLE referrals \
               ADD COLUMN IF NOT EXISTS triaged_by text, \
               ADD COLUMN IF NOT EXISTS triaged_at timestamptz, \
               ADD COLUMN IF NOT EXISTS redirected_to_service text, \
               ADD COLUMN IF NOT EXISTS redirected_referral_id uuid REFERENCES referrals(id)",
            "ALTER TABLE referrals ADD CONSTRAINT referrals_status_chk CHECK (status IN \
               ('received','waitlisted','booked','closed','returned_with_advice','redirected'))",
            // A triaged referral says who and when; a redirected one also says where and which referral.
            "ALTER TABLE referrals ADD CONSTRAINT referrals_triage_chk CHECK ( \
               ((status IN ('returned_with_advice','redirected')) = (triaged_by IS NOT NULL AND triaged_at IS NOT NULL)) \
               AND ((status = 'redirected') = (redirected_to_service IS NOT NULL AND redirected_referral_id IS NOT NULL)))",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    /// Triaged referrals become `closed` (the triage record is dropped with its columns): there is no older
    /// status that means "returned" or "redirected".
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_triage_chk",
            "UPDATE referrals SET status = 'closed' WHERE status IN ('returned_with_advice','redirected')",
            "ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_status_chk",
            "ALTER TABLE referrals \
               DROP COLUMN IF EXISTS redirected_referral_id, DROP COLUMN IF EXISTS redirected_to_service, \
               DROP COLUMN IF EXISTS triaged_at, DROP COLUMN IF EXISTS triaged_by",
            "ALTER TABLE referrals ADD CONSTRAINT referrals_status_chk CHECK (status IN \
               ('received','waitlisted','booked','closed'))",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

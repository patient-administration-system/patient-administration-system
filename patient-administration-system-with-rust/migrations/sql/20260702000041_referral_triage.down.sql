ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_triage_chk;

UPDATE referrals SET status = 'closed' WHERE status IN ('returned_with_advice','redirected');

ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_status_chk;

ALTER TABLE referrals DROP COLUMN IF EXISTS redirected_referral_id, DROP COLUMN IF EXISTS redirected_to_service, DROP COLUMN IF EXISTS triaged_at, DROP COLUMN IF EXISTS triaged_by;

ALTER TABLE referrals ADD CONSTRAINT referrals_status_chk CHECK (status IN ('received','waitlisted','booked','closed'));

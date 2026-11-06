DROP INDEX IF EXISTS idx_appointments_referral;

ALTER TABLE appointments DROP COLUMN IF EXISTS referral_id;

DROP INDEX IF EXISTS idx_referrals_open;

DROP INDEX IF EXISTS ux_referrals_source_ref;

ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_external_ref_chk, DROP CONSTRAINT IF EXISTS referrals_status_chk, DROP CONSTRAINT IF EXISTS referrals_source_chk;

ALTER TABLE referrals DROP COLUMN IF EXISTS updated_at, DROP COLUMN IF EXISTS status, DROP COLUMN IF EXISTS external_ref, DROP COLUMN IF EXISTS source;

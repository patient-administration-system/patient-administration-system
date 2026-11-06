-- A consultant's triage of a referral (NEW-11): the referral can be returned to the GP with advice, or
-- redirected to another service, before anything is booked. Two new statuses, who triaged and when, the
-- service it went to and the new referral that carries the patient there (a redirect never loses the
-- patient). The advice itself is clinical and stays in the clinical record.

ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_status_chk;

ALTER TABLE referrals ADD COLUMN IF NOT EXISTS triaged_by text, ADD COLUMN IF NOT EXISTS triaged_at timestamptz, ADD COLUMN IF NOT EXISTS redirected_to_service text, ADD COLUMN IF NOT EXISTS redirected_referral_id uuid REFERENCES referrals(id);

ALTER TABLE referrals ADD CONSTRAINT referrals_status_chk CHECK (status IN ('received','waitlisted','booked','closed','returned_with_advice','redirected'));

ALTER TABLE referrals ADD CONSTRAINT referrals_triage_chk CHECK ( ((status IN ('returned_with_advice','redirected')) = (triaged_by IS NOT NULL AND triaged_at IS NOT NULL)) AND ((status = 'redirected') = (redirected_to_service IS NOT NULL AND redirected_referral_id IS NOT NULL)));

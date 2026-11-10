-- The link from an electronic patient record (WPAS-9, FR-222, FR-223).
--
-- An EPR sends a referral with its own id as external_ref under source 'epr' (unique per source, so a retried
-- send is the same referral). A clinician may withdraw a suspicion: 'withdrawn' is a stop of its own, neither a
-- downgrade (the suspicion was not ruled out) nor an entry in error (it was real).

ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_source_chk;
ALTER TABLE referrals ADD CONSTRAINT referrals_source_chk CHECK (source IN ('gp','e-rs','internal','epr'));

ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_stop_reason_check;
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_stop_reason_check CHECK (stop_reason IS NULL OR stop_reason IN ('treatment_started','downgraded','died','withdrawn','entered_in_error'));

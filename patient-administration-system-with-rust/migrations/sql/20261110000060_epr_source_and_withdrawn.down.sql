ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_stop_reason_check;
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_stop_reason_check CHECK (stop_reason IS NULL OR stop_reason IN ('treatment_started','downgraded','died','entered_in_error')) NOT VALID;
ALTER TABLE referrals DROP CONSTRAINT IF EXISTS referrals_source_chk;
ALTER TABLE referrals ADD CONSTRAINT referrals_source_chk CHECK (source IN ('gp','e-rs','internal')) NOT VALID;

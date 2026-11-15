-- Back to the fixed method list with `nhs_login`. A digital identity proofing with any other provider has no
-- method to go back to, so the last statement fails while one exists, rather than losing what it said.
ALTER TABLE identity_proofings DROP CONSTRAINT IF EXISTS identity_proofings_provider_check;
ALTER TABLE identity_proofings DROP CONSTRAINT IF EXISTS identity_proofings_method_check;
UPDATE identity_proofings SET method = 'nhs_login' WHERE method = 'digital_identity' AND provider = 'NHS login';
ALTER TABLE identity_proofings DROP COLUMN IF EXISTS provider;
ALTER TABLE identity_proofings ADD CONSTRAINT identity_proofings_method_check
    CHECK (method IN ('in_person','video_id_check','nhs_login','postal_code'));

-- QUAL-40 finding 2: identity proofing named one country's login, `nhs_login`, as a method of its own. A
-- national or regional digital identity scheme is now the method `digital_identity`, and the proofing names
-- the scheme in `provider`, so no scheme is built in. A proofing made with `nhs_login` keeps what it said, as
-- `digital_identity` with the provider 'NHS login'. `provider` is required for `digital_identity` and refused
-- for every other method. The method list is `identity::METHODS`; `tests/qual28_state_lists_test.rs` keeps the
-- two the same.
ALTER TABLE identity_proofings ADD COLUMN IF NOT EXISTS provider text NULL;
ALTER TABLE identity_proofings DROP CONSTRAINT IF EXISTS identity_proofings_method_check;
UPDATE identity_proofings SET method = 'digital_identity', provider = 'NHS login' WHERE method = 'nhs_login';
ALTER TABLE identity_proofings ADD CONSTRAINT identity_proofings_method_check
    CHECK (method IN ('in_person','video_id_check','digital_identity','postal_code'));
ALTER TABLE identity_proofings ADD CONSTRAINT identity_proofings_provider_check
    CHECK ((method = 'digital_identity') = (provider IS NOT NULL)
           AND (provider IS NULL OR length(btrim(provider)) BETWEEN 1 AND 100));

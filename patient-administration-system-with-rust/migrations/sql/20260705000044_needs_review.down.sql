ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_needs_review_chk;

ALTER TABLE patient_communication_profiles DROP COLUMN IF EXISTS needs_reviewed_by, DROP COLUMN IF EXISTS needs_reviewed_at;

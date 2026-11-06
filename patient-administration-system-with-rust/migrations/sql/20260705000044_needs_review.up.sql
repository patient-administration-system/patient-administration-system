-- When a patient's communication needs were last reviewed, and by whom (NEW-20, the Accessible Information
-- Standard's sixth step). Both or neither.

ALTER TABLE patient_communication_profiles ADD COLUMN IF NOT EXISTS needs_reviewed_at timestamptz, ADD COLUMN IF NOT EXISTS needs_reviewed_by text;

ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_needs_review_chk CHECK ((needs_reviewed_at IS NULL AND needs_reviewed_by IS NULL) OR (needs_reviewed_at IS NOT NULL AND length(btrim(needs_reviewed_by)) > 0));

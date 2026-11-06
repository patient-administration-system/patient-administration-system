ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_accessibility_needs_check;

ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_accessibility_needs_check CHECK (accessibility_needs <@ ARRAY['easy_read','large_print','audio','british_sign_language']);

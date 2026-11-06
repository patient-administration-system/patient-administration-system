-- CAS-10: a patient who can only be reached by letter.
--
-- `letter_only` joins the accessibility needs a profile can record. Once set, no text, WhatsApp, email or
-- portal message is generated for the patient: the choke point every message passes through refuses it.

ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_accessibility_needs_check;

ALTER TABLE patient_communication_profiles ADD CONSTRAINT patient_communication_profiles_accessibility_needs_check CHECK (accessibility_needs <@ ARRAY['easy_read','large_print','audio','british_sign_language','letter_only']);

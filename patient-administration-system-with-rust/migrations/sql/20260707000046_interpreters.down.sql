DROP TABLE IF EXISTS interpreter_requests;

ALTER TABLE patient_communication_profiles DROP CONSTRAINT IF EXISTS patient_communication_profiles_interpreter_chk;

ALTER TABLE patient_communication_profiles DROP COLUMN IF EXISTS interpreter_language, DROP COLUMN IF EXISTS interpreter_kind;

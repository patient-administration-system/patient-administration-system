DROP INDEX IF EXISTS idx_audit_log_entity;

DROP INDEX IF EXISTS idx_generated_letters_person;

ALTER TABLE generated_letters DROP COLUMN IF EXISTS trigger_ref, DROP COLUMN IF EXISTS trigger;

ALTER TABLE audit_log DROP COLUMN IF EXISTS context, DROP COLUMN IF EXISTS reason_code;

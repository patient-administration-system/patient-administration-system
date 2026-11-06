DROP INDEX IF EXISTS ux_slots_template_start;

ALTER TABLE slots DROP CONSTRAINT IF EXISTS chk_slots_slot_type;

ALTER TABLE slots DROP COLUMN IF EXISTS template_version, DROP COLUMN IF EXISTS template_id, DROP COLUMN IF EXISTS ring_fenced, DROP COLUMN IF EXISTS slot_type;

DROP TABLE IF EXISTS session_template_versions;

DROP TABLE IF EXISTS session_templates;

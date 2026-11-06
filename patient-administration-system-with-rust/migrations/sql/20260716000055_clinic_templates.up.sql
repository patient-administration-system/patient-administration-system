-- Clinic session templates and slot types (NEW-8).
--
-- A template belongs to a schedule and is versioned: every change is a new version that a second, named
-- person approves before it can generate slots. A version says which weekday, what time of day, how long a
-- slot lasts and the run of slot types (with ring-fencing) that make up one session. Slots made from a
-- template remember which one, so generating again for the same days adds nothing.

CREATE TABLE IF NOT EXISTS session_templates ( id uuid PRIMARY KEY, schedule_id uuid NOT NULL REFERENCES schedules(id), name text NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 100), created_by text NOT NULL CHECK (length(btrim(created_by)) > 0), created_at timestamptz NOT NULL DEFAULT now());

CREATE INDEX IF NOT EXISTS idx_session_templates_schedule ON session_templates (schedule_id);

CREATE TABLE IF NOT EXISTS session_template_versions ( template_id uuid NOT NULL REFERENCES session_templates(id), version integer NOT NULL CHECK (version >= 1), weekday smallint NOT NULL CHECK (weekday BETWEEN 1 AND 7), start_time time NOT NULL, slot_minutes integer NOT NULL CHECK (slot_minutes BETWEEN 5 AND 240), effective_from date NOT NULL, lines jsonb NOT NULL, proposed_by text NOT NULL CHECK (length(btrim(proposed_by)) > 0), proposed_at timestamptz NOT NULL DEFAULT now(), approved_by text NULL, approved_at timestamptz NULL, PRIMARY KEY (template_id, version), CONSTRAINT chk_template_version_approval CHECK ( (approved_by IS NULL AND approved_at IS NULL) OR (approved_by IS NOT NULL AND approved_at IS NOT NULL AND length(btrim(approved_by)) > 0 AND lower(btrim(approved_by)) <> lower(btrim(proposed_by)))));

ALTER TABLE slots ADD COLUMN IF NOT EXISTS slot_type text NULL, ADD COLUMN IF NOT EXISTS ring_fenced boolean NOT NULL DEFAULT false, ADD COLUMN IF NOT EXISTS template_id uuid NULL REFERENCES session_templates(id), ADD COLUMN IF NOT EXISTS template_version integer NULL;

ALTER TABLE slots DROP CONSTRAINT IF EXISTS chk_slots_slot_type;

ALTER TABLE slots ADD CONSTRAINT chk_slots_slot_type CHECK (slot_type IS NULL OR slot_type IN ('new','follow_up','urgent','pifu'));

CREATE UNIQUE INDEX IF NOT EXISTS ux_slots_template_start ON slots (template_id, start_datetime) WHERE template_id IS NOT NULL;

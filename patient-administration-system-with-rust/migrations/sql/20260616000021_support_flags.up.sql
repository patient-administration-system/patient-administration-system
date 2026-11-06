-- CAP-10: flags that can only add help.
--
-- A support flag asks a person to do something *for* a patient: phone them (`needs_support_call`) or
-- refer them to a patient navigator (`navigator_referral`). There is deliberately no column here, and a
-- guard test keeps it so for the whole schema, that scores, ranks, penalises or suppresses contact.

CREATE TABLE IF NOT EXISTS support_flags ( id uuid PRIMARY KEY, person_id uuid NOT NULL, appointment_id uuid NULL, kind text NOT NULL CHECK (kind IN ('needs_support_call','navigator_referral')), reason_code text NULL, created_by text NOT NULL CHECK (length(btrim(created_by)) > 0), created_at timestamptz NOT NULL DEFAULT now(), resolved_at timestamptz NULL, resolved_by text NULL, outcome text NULL CHECK (outcome IS NULL OR outcome IN ('helped','no_answer','declined_help','not_needed')), CHECK ((resolved_at IS NULL AND resolved_by IS NULL AND outcome IS NULL) OR (resolved_at IS NOT NULL AND outcome IS NOT NULL AND length(btrim(resolved_by)) > 0)));

CREATE INDEX IF NOT EXISTS idx_support_flags_open ON support_flags (created_at) WHERE resolved_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_support_flags_person ON support_flags (person_id);

-- TSC-8: a patient we tried every way and heard nothing back from, as work for a person to try by phone.

ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_kind_check;

ALTER TABLE work_items ADD CONSTRAINT work_items_kind_check CHECK (kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached'));

-- What came of the phone attempt an `unreached` item asked for; the item's source_id is the fact it stands for.
CREATE TABLE IF NOT EXISTS unreached_call_outcomes (
    source_id uuid PRIMARY KEY,
    outcome text NOT NULL CHECK (outcome IN ('reached','no_answer','wrong_number')),
    recorded_by text NOT NULL CHECK (length(btrim(recorded_by)) > 0),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

-- TSC-10: a clinician's or a room's unavailability as a time window. No reason and no detail are kept.
-- The free slots it covers are blocked; each booked appointment it touches becomes work for a person to
-- rebook with the patient. Nothing is cancelled and the patient hears nothing until a person acts.

CREATE TABLE IF NOT EXISTS staff_absences (
    id uuid PRIMARY KEY,
    owner_kind text NOT NULL CHECK (owner_kind IN ('practitioner','room')),
    owner_id uuid NOT NULL,
    starts_at timestamptz NOT NULL,
    ends_at timestamptz NOT NULL,
    recorded_by text NOT NULL CHECK (char_length(btrim(recorded_by)) BETWEEN 1 AND 200),
    recorded_at timestamptz NOT NULL DEFAULT now(),
    withdrawn_by text NULL,
    withdrawn_at timestamptz NULL,
    CHECK (ends_at > starts_at),
    CHECK ((withdrawn_at IS NULL) = (withdrawn_by IS NULL))
);

CREATE INDEX IF NOT EXISTS staff_absences_owner_idx ON staff_absences (owner_kind, owner_id) WHERE withdrawn_at IS NULL;

-- The free slots an absence blocked, so withdrawing it frees those and no others.
CREATE TABLE IF NOT EXISTS absence_blocked_slots (
    absence_id uuid NOT NULL REFERENCES staff_absences(id),
    slot_id uuid NOT NULL,
    PRIMARY KEY (absence_id, slot_id)
);

-- The booked appointments an absence touched when it was recorded: each is a work item's source.
CREATE TABLE IF NOT EXISTS absence_appointments (
    id uuid PRIMARY KEY,
    absence_id uuid NOT NULL REFERENCES staff_absences(id),
    appointment_id uuid NOT NULL,
    UNIQUE (absence_id, appointment_id)
);

ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_kind_check;

ALTER TABLE work_items ADD CONSTRAINT work_items_kind_check CHECK (kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached','absence_rebook'));

ALTER TABLE work_views DROP CONSTRAINT IF EXISTS work_views_kind_check;

ALTER TABLE work_views ADD CONSTRAINT work_views_kind_check CHECK (kind IS NULL OR kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached','absence_rebook'));

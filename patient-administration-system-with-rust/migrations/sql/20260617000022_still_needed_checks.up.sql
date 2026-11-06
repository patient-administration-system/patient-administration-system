-- CAP-8: "do you still need this?" checks for long waiters.
--
-- One row each time a person asks a patient on a long-waiting pathway whether they still need the
-- appointment, and what they said. The answer is recorded and audited; it never removes the patient from a
-- list or stops their clock by itself (a person decides what follows).

CREATE TABLE IF NOT EXISTS still_needed_checks ( id uuid PRIMARY KEY, pathway_id uuid NOT NULL, person_id uuid NOT NULL, asked_by text NOT NULL CHECK (length(btrim(asked_by)) > 0), asked_at timestamptz NOT NULL DEFAULT now(), answer text NULL CHECK (answer IS NULL OR answer IN ('still_needed','no_longer_needed','unsure','no_reply')), recorded_by text NULL, answered_at timestamptz NULL, CHECK ((answer IS NULL AND recorded_by IS NULL AND answered_at IS NULL) OR (answer IS NOT NULL AND answered_at IS NOT NULL AND length(btrim(recorded_by)) > 0)));

CREATE INDEX IF NOT EXISTS idx_still_needed_pathway ON still_needed_checks (pathway_id, asked_at DESC);

-- CORE-4: when a patient arrived and left, and where they are now.
--
-- `arrived_at` and `completed_at` are stamped by check-in and complete; `current_room` is a short label a
-- person sets when they move the patient (a move is audited). The in-facility list is everyone arrived and
-- not yet completed.

ALTER TABLE appointments ADD COLUMN IF NOT EXISTS arrived_at timestamptz NULL, ADD COLUMN IF NOT EXISTS completed_at timestamptz NULL, ADD COLUMN IF NOT EXISTS current_room text NULL;

ALTER TABLE appointments ADD CONSTRAINT appointments_current_room_chk CHECK (current_room IS NULL OR (length(btrim(current_room)) BETWEEN 1 AND 60));

CREATE INDEX IF NOT EXISTS idx_appointments_in_facility ON appointments (arrived_at) WHERE status = 'arrived';

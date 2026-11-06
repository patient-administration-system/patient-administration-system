-- CAP-2: every reply that arrives from outside (confirm, cancel, rebook) is stored, whether it
-- was applied or refused. `message_id` is unique, which is what makes delivery idempotent: the
-- second delivery of a message finds the first one's outcome instead of applying it again.

CREATE TABLE IF NOT EXISTS inbound_replies ( id uuid PRIMARY KEY, message_id text NOT NULL UNIQUE, action text NOT NULL CHECK (action IN ('confirm','cancel','rebook')), appointment_id uuid NOT NULL, person_id uuid NOT NULL, channel text NULL, barrier_code text NULL, new_slot_id uuid NULL, status text NOT NULL CHECK (status IN ('applied','rejected')), reject_reason text NULL, new_appointment_id uuid NULL, received_at timestamptz NOT NULL, created_at timestamptz NOT NULL DEFAULT now());

CREATE INDEX IF NOT EXISTS ix_inbound_replies_appointment ON inbound_replies (appointment_id);

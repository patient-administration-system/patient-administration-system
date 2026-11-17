-- A resent HL7 v2 message is not processed twice (NFR-31, HUB-2). One row per message that changed, or is
-- changing, something: the sender's application and facility and a SHA-256 of its control id (MSH-10), never
-- the id itself, which some senders build from a patient identifier. `processing` until it is answered; `done`
-- with the AA it was answered with, which a resend gets again. Kept PAS_HL7_DEDUPE_DAYS, then removed.
CREATE TABLE IF NOT EXISTS hl7_inbound_messages (
    sending_app TEXT NOT NULL,
    sending_facility TEXT NOT NULL,
    control_id_sha256 TEXT NOT NULL CHECK (control_id_sha256 ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK (state IN ('processing', 'done')),
    ack_code TEXT CHECK (ack_code IN ('AA', 'AE', 'AR')),
    ack_text TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    PRIMARY KEY (sending_app, sending_facility, control_id_sha256)
);
CREATE INDEX IF NOT EXISTS idx_hl7_inbound_messages_received ON hl7_inbound_messages (received_at);

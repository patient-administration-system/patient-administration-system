-- PILOT-8: a time-limited offer of a released slot to one standby patient at a time.
--
-- One row per offer. The accept/decline link carries an opaque single-use token; only its SHA-256 is stored.
-- An offer is `open` until it is accepted, declined, expires, or the slot is taken some other way (`missed`).

CREATE TABLE IF NOT EXISTS slot_offers ( id uuid PRIMARY KEY, release_id uuid NOT NULL, slot_id uuid NOT NULL, standby_entry_id uuid NOT NULL, person_id uuid NOT NULL, token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'), status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','accepted','declined','expired','missed')), offered_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL, answered_at timestamptz NULL, appointment_id uuid NULL, message_sent boolean NOT NULL DEFAULT false);

CREATE UNIQUE INDEX IF NOT EXISTS ux_slot_offers_open_release ON slot_offers (release_id) WHERE status = 'open';

CREATE UNIQUE INDEX IF NOT EXISTS ux_slot_offers_person_release ON slot_offers (release_id, person_id);

CREATE INDEX IF NOT EXISTS idx_slot_offers_due ON slot_offers (expires_at) WHERE status = 'open';

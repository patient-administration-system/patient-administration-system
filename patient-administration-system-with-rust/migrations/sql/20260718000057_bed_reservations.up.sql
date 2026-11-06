-- Who a reserved bed is held for, until when, and what happened to the hold (OFH-4).
--
-- `pre_admit` used to flip a bed to `Reserved` and record nothing else, so a pre-admission nobody followed up
-- kept the bed out of use for ever. A reservation names the person, the planned encounter and the member of
-- staff who made it, and may carry an `expires_at`; a sweep releases a lapsed one. `bed_occupancy_events` is
-- the list of what happened to a bed's hold: reserved, cancelled, expired.

CREATE TABLE IF NOT EXISTS bed_reservations ( id uuid PRIMARY KEY, bed_id uuid NOT NULL REFERENCES beds (id), person_id uuid NOT NULL, encounter_id uuid NOT NULL REFERENCES encounters (id), held_by text NULL, created_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NULL, released_at timestamptz NULL, release_reason text NULL, CONSTRAINT bed_reservations_reason_chk CHECK ( (released_at IS NULL) = (release_reason IS NULL) AND (release_reason IS NULL OR release_reason IN ('cancelled','expired'))), CONSTRAINT bed_reservations_expiry_chk CHECK (expires_at IS NULL OR expires_at > created_at));

CREATE UNIQUE INDEX IF NOT EXISTS bed_reservations_one_open_per_bed ON bed_reservations (bed_id) WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS bed_reservations_open_expiry ON bed_reservations (expires_at) WHERE released_at IS NULL AND expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS bed_occupancy_events ( id uuid PRIMARY KEY, bed_id uuid NOT NULL REFERENCES beds (id), event text NOT NULL CHECK (event IN ('reserved','cancelled','expired')), reservation_id uuid NULL REFERENCES bed_reservations (id), person_id uuid NULL, actor text NULL, at timestamptz NOT NULL DEFAULT now());

CREATE INDEX IF NOT EXISTS bed_occupancy_events_bed_at ON bed_occupancy_events (bed_id, at DESC);

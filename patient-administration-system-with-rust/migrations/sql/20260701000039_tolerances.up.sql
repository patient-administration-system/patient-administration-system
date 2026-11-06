-- Tolerances: how far a measure may drift from its target before it is amber, and before it is red.
--
-- One row per measure, and only where someone has changed the built-in default. A named person sets it.

CREATE TABLE IF NOT EXISTS tolerances ( metric text PRIMARY KEY, target double precision NOT NULL, tolerance double precision NOT NULL CHECK (tolerance >= 0), escalate_beyond double precision NOT NULL, set_by text NOT NULL CHECK (length(btrim(set_by)) > 0), note text NOT NULL DEFAULT '', updated_at timestamptz NOT NULL DEFAULT now(), CHECK (escalate_beyond >= tolerance));

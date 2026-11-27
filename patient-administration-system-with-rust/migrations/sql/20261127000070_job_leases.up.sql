-- One row per background job: which run holds it and until when (audit PA-09). A run takes the lease in one
-- short statement and lets the connection go, so the job's own queries can use the whole pool, even a pool of
-- one. A run that ends releases it; a process that dies leaves it to lapse at held_until. Holds no personal data.
CREATE TABLE IF NOT EXISTS job_leases (
    name text PRIMARY KEY CHECK (length(btrim(name)) > 0),
    holder uuid NOT NULL,
    held_until timestamptz NOT NULL,
    taken_at timestamptz NOT NULL DEFAULT now()
);

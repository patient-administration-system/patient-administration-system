-- `Idempotency-Key` on POSTs (HUB-8): a client that retries a request it did not hear back from gets the first
-- answer, and nothing is created twice. One row per caller and key, holding a hash of the request, and the
-- answer once there is one. Kept 24 hours.
CREATE TABLE IF NOT EXISTS idempotency_keys (
    caller TEXT NOT NULL,
    key_sha256 TEXT NOT NULL CHECK (key_sha256 ~ '^[0-9a-f]{64}$'),
    request_sha256 TEXT NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK (state IN ('processing', 'done')),
    status INTEGER,
    content_type TEXT,
    body TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (caller, key_sha256)
);
CREATE INDEX IF NOT EXISTS idx_idempotency_keys_created ON idempotency_keys (created_at);

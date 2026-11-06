-- Event subscriptions (NEW-4): who wants to hear about appointment and waiting-list changes, and one
-- delivery row per subscriber per event so a change reaches each subscriber once, is retried after a failure
-- and survives a restart. The event is copied into the delivery because the outbox row can be moved to the
-- dead-letter queue.

CREATE TABLE IF NOT EXISTS subscriptions ( id uuid PRIMARY KEY, topic text NOT NULL CHECK (topic IN ('appointment','waiting-list')), endpoint text NOT NULL CHECK (endpoint ~ '^https?://' AND length(endpoint) <= 500), secret_env text CHECK (secret_env IS NULL OR secret_env ~ '^PAS_SUB_SECRET_[A-Z0-9_]+$'), status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','off')), created_by text NOT NULL CHECK (length(btrim(created_by)) > 0), created_at timestamptz NOT NULL DEFAULT now(), ended_at timestamptz);

CREATE TABLE IF NOT EXISTS subscription_deliveries ( id uuid PRIMARY KEY, subscription_id uuid NOT NULL REFERENCES subscriptions (id), outbox_event_id uuid NOT NULL, event_type text NOT NULL, payload jsonb NOT NULL, event_at timestamptz NOT NULL, status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','delivered','failed')), attempts integer NOT NULL DEFAULT 0, next_attempt_at timestamptz NOT NULL, last_error text, delivered_at timestamptz, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE (subscription_id, outbox_event_id));

CREATE INDEX IF NOT EXISTS idx_subscription_deliveries_due ON subscription_deliveries (next_attempt_at) WHERE status = 'pending';

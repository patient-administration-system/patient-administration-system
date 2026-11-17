DROP INDEX IF EXISTS idx_outbox_dead_letters_aggregate;
ALTER TABLE outbox_dead_letters DROP COLUMN IF EXISTS aggregate_key;
DROP INDEX IF EXISTS idx_outbox_events_pending_at;
DROP INDEX IF EXISTS idx_outbox_events_pending_aggregate;
ALTER TABLE outbox_events DROP COLUMN IF EXISTS aggregate_key;
ALTER TABLE outbox_events DROP COLUMN IF EXISTS next_attempt_at;

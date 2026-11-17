-- Outbox delivery by failure class, with backoff and per-record order (NFR-32, HUB-4).
--
-- Adding a stored generated column rewrites `outbox_events`: on a large outbox this takes a while and holds a
-- lock, so run it when the dispatcher can wait.
--
-- `next_attempt_at`: a deferred event is not tried again before this (5 s, doubling, at most 15 min, or the
-- receiver's Retry-After). NULL means at once.
--
-- `aggregate_key`: the record an event is about, so its events are delivered in order: the appointment when the
-- payload names one (booked and cancelled share it), else the person, the encounter, the admission or the
-- account. NULL (none of these) is delivered in no particular order with the rest. An event waits while an
-- earlier event with its key is undelivered, and while a dead letter with its key is held.
ALTER TABLE outbox_events ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;
ALTER TABLE outbox_events ADD COLUMN IF NOT EXISTS aggregate_key TEXT GENERATED ALWAYS AS (
    CASE
        WHEN payload ? 'appointment_id' THEN 'appointment:' || (payload->>'appointment_id')
        WHEN payload ? 'person_id' THEN 'person:' || (payload->>'person_id')
        WHEN payload ? 'encounter_id' THEN 'encounter:' || (payload->>'encounter_id')
        WHEN payload ? 'admission_id' THEN 'admission:' || (payload->>'admission_id')
        WHEN payload ? 'account_id' THEN 'account:' || (payload->>'account_id')
    END
) STORED;
CREATE INDEX IF NOT EXISTS idx_outbox_events_pending_aggregate
    ON outbox_events (aggregate_key, at, id) WHERE published = false;
-- The dispatcher reads the undelivered events oldest first.
CREATE INDEX IF NOT EXISTS idx_outbox_events_pending_at ON outbox_events (at, id) WHERE published = false;

ALTER TABLE outbox_dead_letters ADD COLUMN IF NOT EXISTS aggregate_key TEXT GENERATED ALWAYS AS (
    CASE
        WHEN payload ? 'appointment_id' THEN 'appointment:' || (payload->>'appointment_id')
        WHEN payload ? 'person_id' THEN 'person:' || (payload->>'person_id')
        WHEN payload ? 'encounter_id' THEN 'encounter:' || (payload->>'encounter_id')
        WHEN payload ? 'admission_id' THEN 'admission:' || (payload->>'admission_id')
        WHEN payload ? 'account_id' THEN 'account:' || (payload->>'account_id')
    END
) STORED;
CREATE INDEX IF NOT EXISTS idx_outbox_dead_letters_aggregate ON outbox_dead_letters (aggregate_key);

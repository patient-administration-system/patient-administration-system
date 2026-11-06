UPDATE inbound_texts SET pending_intent = 'reschedule' WHERE pending_intent = 'rebook';

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_pending_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_pending_intent_check CHECK (pending_intent IS NULL OR pending_intent IN ('confirm','cancel','reschedule'));

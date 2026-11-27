-- Undo TSC-11. A reply that refused or resumed surveys is kept, read as `unknown` (the older lists have no
-- word for it); the refusals themselves are dropped, so check with the patient before rolling back.

UPDATE inbound_texts SET intent = 'unknown' WHERE intent IN ('stop_surveys','start_surveys');

UPDATE inbound_texts SET outcome = 'unknown' WHERE outcome IN ('stop_kind','start_kind');

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check CHECK (intent IN ('confirm','cancel','reschedule','stop','start','help','choice','unknown'));

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','start','help','unknown','unmatched','ambiguous_number','no_open_reminder'));

DROP TABLE IF EXISTS template_message_kinds;

DROP TABLE IF EXISTS message_kind_refusals;

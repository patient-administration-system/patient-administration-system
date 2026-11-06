DROP TABLE IF EXISTS message_optouts;

DELETE FROM inbound_texts WHERE intent = 'start' OR outcome = 'start';

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check CHECK (intent IN ('confirm','cancel','reschedule','stop','help','choice','unknown'));

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','help','unknown','unmatched','ambiguous_number','no_open_reminder'));

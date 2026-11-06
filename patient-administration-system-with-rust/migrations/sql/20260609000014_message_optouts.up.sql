-- PILOT-7: per-channel opt-outs, and the inbound-text ledger learns START.
--
-- An opt-out is held against a *subject*: `person:<uuid>` when we know who asked, and `address:<number
-- or email>` always (a STOP from a shared or unknown number must still stop sends to that number). A
-- send is blocked if either subject has an active opt-out for the channel. `resumed_at` is set when the
-- patient texts START; the row is kept so the history is not lost.

CREATE TABLE IF NOT EXISTS message_optouts ( id uuid PRIMARY KEY, subject text NOT NULL CHECK (subject ~ '^(person|address):.+'), channel text NOT NULL CHECK (channel IN ('sms','whatsapp','apple_messages','email','voice')), opted_out_at timestamptz NOT NULL DEFAULT now(), resumed_at timestamptz NULL, source text NOT NULL CHECK (source IN ('reply_stop','staff','portal')), recorded_by text NULL, UNIQUE (subject, channel));

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check CHECK (intent IN ('confirm','cancel','reschedule','stop','start','help','choice','unknown'));

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','start','help','unknown','unmatched','ambiguous_number','no_open_reminder'));

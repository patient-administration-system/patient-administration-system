-- TSC-11: kinds of message a patient can refuse on their own, apart from the channel opt-out (STOP).
-- A service message about an appointment (booking, change, reminder) has no kind here: only STOP governs it.

CREATE TABLE IF NOT EXISTS message_kind_refusals (
    subject text NOT NULL CHECK (subject ~ '^(person|address):.+'),
    kind text NOT NULL CHECK (kind IN ('survey','experiment')),
    refused_at timestamptz NOT NULL DEFAULT now(),
    resumed_at timestamptz NULL,
    source text NOT NULL CHECK (source IN ('reply_stop','staff','portal')),
    recorded_by text NULL,
    PRIMARY KEY (subject, kind)
);

-- A template group (every language variant of one name) that is not a service message.
CREATE TABLE IF NOT EXISTS template_message_kinds (
    template_name text PRIMARY KEY CHECK (char_length(template_name) BETWEEN 1 AND 200),
    kind text NOT NULL CHECK (kind IN ('survey')),
    set_by text NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check CHECK (intent IN ('confirm','cancel','reschedule','stop','start','help','choice','unknown','stop_surveys','start_surveys'));

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','start','help','unknown','unmatched','ambiguous_number','no_open_reminder','stop_kind','start_kind'));

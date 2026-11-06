-- CAP-5: the short list of alternative slots a patient was offered.
--
-- A patient may only rebook into a slot from an offer the PAS made for that appointment, so the offer
-- is stored: choosing a slot that is not in it is refused, and an offer can be used once. Slot ids are
-- kept as a JSON array; the offer is short-lived and is not queried by slot.

CREATE TABLE IF NOT EXISTS rebook_offers ( id uuid PRIMARY KEY, appointment_id uuid NOT NULL, person_id uuid NOT NULL, slot_ids jsonb NOT NULL, expires_at timestamptz NOT NULL, chosen_slot_id uuid NULL, chosen_at timestamptz NULL, created_at timestamptz NOT NULL DEFAULT now());

CREATE INDEX IF NOT EXISTS idx_rebook_offers_appointment ON rebook_offers (appointment_id, created_at DESC);

ALTER TABLE reply_tokens DROP CONSTRAINT IF EXISTS reply_tokens_consumed_action_check;

ALTER TABLE reply_tokens ADD CONSTRAINT reply_tokens_consumed_action_check CHECK (consumed_action IS NULL OR consumed_action IN ('confirm','cancel','reschedule','rebook'));

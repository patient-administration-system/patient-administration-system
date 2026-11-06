DROP TABLE IF EXISTS rebook_offers;

ALTER TABLE reply_tokens DROP CONSTRAINT IF EXISTS reply_tokens_consumed_action_check;

ALTER TABLE reply_tokens ADD CONSTRAINT reply_tokens_consumed_action_check CHECK (consumed_action IS NULL OR consumed_action IN ('confirm','cancel','reschedule'));

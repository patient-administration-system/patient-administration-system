-- A "different time" text reply is answered with real alternatives (CAP-5), so a question the PAS has put to
-- the patient can now be "rebook": the stored candidates are the appointment followed by the offered slots.

ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_pending_intent_check;

ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_pending_intent_check CHECK (pending_intent IS NULL OR pending_intent IN ('confirm','cancel','reschedule','rebook'));

DROP TABLE IF EXISTS virtual_ok_services;

ALTER TABLE inbound_replies DROP CONSTRAINT IF EXISTS inbound_replies_action_check;

ALTER TABLE inbound_replies ADD CONSTRAINT inbound_replies_action_check CHECK (action IN ('confirm','cancel','rebook'));

ALTER TABLE inbound_replies DROP COLUMN IF EXISTS modality;

ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_conversion_barrier_chk, DROP CONSTRAINT IF EXISTS appointments_conversion_state_chk, DROP CONSTRAINT IF EXISTS appointments_modality_chk;

ALTER TABLE appointments DROP COLUMN IF EXISTS converted_at, DROP COLUMN IF EXISTS conversion_barrier_code, DROP COLUMN IF EXISTS conversion_state, DROP COLUMN IF EXISTS modality;

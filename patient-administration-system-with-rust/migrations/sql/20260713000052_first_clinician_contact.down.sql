ALTER TABLE encounters DROP CONSTRAINT IF EXISTS chk_encounters_first_contact;

ALTER TABLE encounters DROP COLUMN IF EXISTS first_contact_by, DROP COLUMN IF EXISTS first_contact_at;

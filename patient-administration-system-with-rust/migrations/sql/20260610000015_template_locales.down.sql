DROP INDEX IF EXISTS uq_letter_templates_active_name_locale;

ALTER TABLE letter_templates DROP CONSTRAINT IF EXISTS letter_templates_locale_chk;

ALTER TABLE letter_templates DROP COLUMN IF EXISTS locale;

ALTER TABLE generated_letters DROP COLUMN IF EXISTS locale;

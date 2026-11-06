-- PILOT-5 / CAP-9: templates carry a language. Templates with the same name are the language variants
-- of one message; only one active variant per name and language. Generated letters remember which
-- variant they used.

ALTER TABLE letter_templates ADD COLUMN IF NOT EXISTS locale text NOT NULL DEFAULT 'en-001';

ALTER TABLE letter_templates ADD CONSTRAINT letter_templates_locale_chk CHECK (locale ~ '^[a-z]{2,3}(-[A-Za-z0-9]{1,8})*$' AND char_length(locale) <= 16);

CREATE UNIQUE INDEX IF NOT EXISTS uq_letter_templates_active_name_locale ON letter_templates (name, locale) WHERE active;

ALTER TABLE generated_letters ADD COLUMN IF NOT EXISTS locale text NULL;

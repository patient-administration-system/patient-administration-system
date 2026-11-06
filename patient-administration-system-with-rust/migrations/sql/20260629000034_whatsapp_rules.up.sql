-- CAP-13: WhatsApp needs an opt-in and an approved template.
--
-- WhatsApp Business messages sent outside a customer-service window must use a template the provider has
-- approved, and only to a person who has opted in to hear from the organisation on WhatsApp. Both are
-- recorded here, by named people, and checked where every message is generated when
-- `PAS_WHATSAPP_REQUIRE_OPTIN=true` (which a production deployment must set before WhatsApp goes live).

CREATE TABLE IF NOT EXISTS whatsapp_optins ( person_id uuid PRIMARY KEY, source text NOT NULL CHECK (source IN ('patient_reply','portal','in_person','paper')), recorded_by text NOT NULL CHECK (length(btrim(recorded_by)) > 0), opted_in_at timestamptz NOT NULL DEFAULT now(), withdrawn_at timestamptz NULL);

CREATE TABLE IF NOT EXISTS whatsapp_template_approvals ( template_id uuid PRIMARY KEY, provider_template_id text NOT NULL CHECK (length(btrim(provider_template_id)) > 0), approved_by text NOT NULL CHECK (length(btrim(approved_by)) > 0), approved_at timestamptz NOT NULL DEFAULT now(), revoked_at timestamptz NULL);

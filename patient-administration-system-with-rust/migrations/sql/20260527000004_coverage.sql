-- Coverage / insurance records (v0.10.0).
--
-- Adds the `coverages` table — one row per insurance / self-pay /
-- other-payer record carried by a patient. Optionally linked to a
-- billing `accounts.id` so invoices can surface payer detail.
--
-- No `deleted_at` column on this table — invariant §5.3 keeps soft-
-- delete to patients/encounters/appointments only. Retirement is via
-- `status = 'cancelled'` or `status = 'entered_in_error'`.

CREATE TABLE IF NOT EXISTS "coverages" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "account_id" uuid NULL, "status" text NOT NULL, "kind" text NOT NULL, "subscriber_id" uuid NULL, "payor_name" text NOT NULL, "payor_identifier" text NULL, "policy_number" text NOT NULL, "group_number" text NULL, "relationship" text NOT NULL, "start_date" date NOT NULL, "end_date" date NULL, "created_at" timestamp with time zone NOT NULL DEFAULT (now()), "updated_at" timestamp with time zone NOT NULL DEFAULT (now()) );

CREATE INDEX "idx_coverages_patient" ON "coverages" ("patient_id");

CREATE INDEX IF NOT EXISTS idx_coverages_account ON coverages (account_id) WHERE account_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_coverages_active_policy ON coverages (patient_id, payor_name, policy_number) WHERE status = 'active';

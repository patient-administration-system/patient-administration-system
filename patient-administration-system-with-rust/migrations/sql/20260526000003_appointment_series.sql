-- Recurring appointment series (v0.9.0).
--
-- Adds the `appointment_series` table (the plan) and a nullable
-- `series_id` column on `appointments` (the link from each occurrence
-- back to its series). Occurrences are concrete `appointments` rows
-- generated at series-create time; they remain singleton rows in every
-- respect except for this backlink.

CREATE TABLE IF NOT EXISTS "appointment_series" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "practitioner_id" uuid NULL, "service_type" text NOT NULL, "start_datetime" timestamp with time zone NOT NULL, "duration_minutes" integer NOT NULL, "rule" jsonb NOT NULL, "status" text NOT NULL, "reason" text NULL, "created_at" timestamp with time zone NOT NULL DEFAULT (now()), "updated_at" timestamp with time zone NOT NULL DEFAULT (now()) );

CREATE INDEX "idx_appointment_series_patient" ON "appointment_series" ("patient_id");

ALTER TABLE "appointments" ADD COLUMN "series_id" uuid NULL;

CREATE INDEX "idx_appointments_series" ON "appointments" ("series_id");

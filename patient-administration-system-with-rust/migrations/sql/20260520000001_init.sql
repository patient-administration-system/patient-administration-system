-- Initial migration for the Patient Administration System (PAS).
--
-- PRAGMATIC SIMPLIFICATION (v0.1):
-- - Nested Vec data is stored inline on parent rows as JSONB columns instead
-- of separate child tables, to keep the schema tractable in v0.1.
-- - `patients` stores `identifiers`, `additional_names`, `telecom`, `addresses`,
-- `emergency_contacts` (and `name`) as JSONB columns.
-- - `practitioners` follows the same pattern for `identifiers`, `telecom`,
-- `addresses`, and `name`.
-- - This eliminates auxiliary tables (`patient_identifiers`, `patient_names`,
-- `patient_addresses`, `patient_contacts`, standalone `emergency_contacts`).
-- - This is a v0.1 trade-off; later versions can normalize.
--
-- Decimal money columns are stored at the DB layer as `decimal(20,4)`.
-- Conversion to/from `crate::models::Money` happens in repositories.

CREATE TABLE IF NOT EXISTS "patients" ( "id" uuid NOT NULL PRIMARY KEY, "mpi_id" uuid NULL, "active" bool NOT NULL DEFAULT TRUE, "name" jsonb NOT NULL, "additional_names" jsonb NOT NULL DEFAULT ('[]'::jsonb), "identifiers" jsonb NOT NULL DEFAULT ('[]'::jsonb), "telecom" jsonb NOT NULL DEFAULT ('[]'::jsonb), "addresses" jsonb NOT NULL DEFAULT ('[]'::jsonb), "gender" text NOT NULL, "birth_date" date NULL, "deceased" bool NOT NULL DEFAULT FALSE, "deceased_datetime" timestamp with time zone NULL, "emergency_contacts" jsonb NOT NULL DEFAULT ('[]'::jsonb), "marital_status" text NULL, "deleted_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "practitioners" ( "id" uuid NOT NULL PRIMARY KEY, "active" bool NOT NULL DEFAULT TRUE, "name" jsonb NOT NULL, "identifiers" jsonb NOT NULL DEFAULT ('[]'::jsonb), "telecom" jsonb NOT NULL DEFAULT ('[]'::jsonb), "addresses" jsonb NOT NULL DEFAULT ('[]'::jsonb), "gender" text NOT NULL, "birth_date" date NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "departments" ( "id" uuid NOT NULL PRIMARY KEY, "facility_id" uuid NOT NULL, "name" text NOT NULL, "code" text NOT NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "practitioner_roles" ( "id" uuid NOT NULL PRIMARY KEY, "practitioner_id" uuid NOT NULL, "department_id" uuid NOT NULL, "role" text NOT NULL, "specialty" text NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "facilities" ( "id" uuid NOT NULL PRIMARY KEY, "name" text NOT NULL, "code" text NOT NULL UNIQUE, "address" jsonb NOT NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "wards" ( "id" uuid NOT NULL PRIMARY KEY, "facility_id" uuid NOT NULL, "name" text NOT NULL, "code" text NOT NULL, "capacity" integer NOT NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL, CONSTRAINT "uq_wards_facility_code" UNIQUE ("facility_id", "code") );

CREATE TABLE IF NOT EXISTS "rooms" ( "id" uuid NOT NULL PRIMARY KEY, "ward_id" uuid NOT NULL, "name" text NOT NULL, "code" text NOT NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL, CONSTRAINT "uq_rooms_ward_code" UNIQUE ("ward_id", "code") );

CREATE TABLE IF NOT EXISTS "beds" ( "id" uuid NOT NULL PRIMARY KEY, "room_id" uuid NOT NULL, "name" text NOT NULL, "code" text NOT NULL, "status" text NOT NULL DEFAULT 'available', "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL, CONSTRAINT "uq_beds_room_code" UNIQUE ("room_id", "code") );

CREATE TABLE IF NOT EXISTS "encounters" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "class" text NOT NULL, "status" text NOT NULL, "period_start" timestamp with time zone NOT NULL, "period_end" timestamp with time zone NULL, "practitioner_id" uuid NULL, "department_id" uuid NULL, "reason" text NULL, "deleted_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "admissions" ( "id" uuid NOT NULL PRIMARY KEY, "encounter_id" uuid NOT NULL, "bed_id" uuid NOT NULL, "admitting_practitioner_id" uuid NULL, "admitted_at" timestamp with time zone NOT NULL, "reason" text NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "transfers" ( "id" uuid NOT NULL PRIMARY KEY, "admission_id" uuid NOT NULL, "from_bed_id" uuid NOT NULL, "to_bed_id" uuid NOT NULL, "reason" text NULL, "transferred_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "discharges" ( "id" uuid NOT NULL PRIMARY KEY, "admission_id" uuid NOT NULL, "discharging_practitioner_id" uuid NULL, "discharged_at" timestamp with time zone NOT NULL, "disposition" text NULL, "notes" text NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "bed_assignments" ( "id" uuid NOT NULL PRIMARY KEY, "encounter_id" uuid NOT NULL, "bed_id" uuid NOT NULL, "assigned_at" timestamp with time zone NOT NULL, "released_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "schedules" ( "id" uuid NOT NULL PRIMARY KEY, "owner_kind" text NOT NULL, "owner_id" uuid NOT NULL, "service_type" text NOT NULL, "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "slots" ( "id" uuid NOT NULL PRIMARY KEY, "schedule_id" uuid NOT NULL, "start_datetime" timestamp with time zone NOT NULL, "end_datetime" timestamp with time zone NOT NULL, "status" text NOT NULL DEFAULT 'free', "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "appointments" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "slot_id" uuid NULL, "practitioner_id" uuid NULL, "start_datetime" timestamp with time zone NOT NULL, "end_datetime" timestamp with time zone NOT NULL, "status" text NOT NULL, "reason" text NULL, "from_waitlist_entry_id" uuid NULL, "cancellation_reason" text NULL, "deleted_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "referrals" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "referring_practitioner_id" uuid NULL, "target_service" text NOT NULL, "reason" text NULL, "received_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "waitlist_entries" ( "id" uuid NOT NULL PRIMARY KEY, "referral_id" uuid NULL, "patient_id" uuid NOT NULL, "target_service" text NOT NULL, "priority" text NOT NULL, "status" text NOT NULL DEFAULT 'waiting', "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "rtt_pathways" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "target_service" text NOT NULL, "breach_weeks" integer NOT NULL DEFAULT 18, "status" text NOT NULL DEFAULT 'active', "started_at" timestamp with time zone NOT NULL, "stopped_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "rtt_clock_events" ( "id" uuid NOT NULL PRIMARY KEY, "pathway_id" uuid NOT NULL, "kind" text NOT NULL, "reason" text NULL, "event_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "letter_templates" ( "id" uuid NOT NULL PRIMARY KEY, "name" text NOT NULL, "subject" text NOT NULL, "body_tera" text NOT NULL, "required_variables" jsonb NOT NULL DEFAULT ('[]'::jsonb), "channels" jsonb NOT NULL DEFAULT ('[]'::jsonb), "active" bool NOT NULL DEFAULT TRUE, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "generated_letters" ( "id" uuid NOT NULL PRIMARY KEY, "template_id" uuid NOT NULL, "patient_id" uuid NOT NULL, "appointment_id" uuid NULL, "rendered_subject" text NOT NULL, "rendered_body" text NOT NULL, "channel" text NOT NULL, "status" text NOT NULL DEFAULT 'pending', "sent_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "accounts" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "status" text NOT NULL DEFAULT 'open', "currency" text NOT NULL, "opened_at" timestamp with time zone NOT NULL, "closed_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "charges" ( "id" uuid NOT NULL PRIMARY KEY, "account_id" uuid NOT NULL, "encounter_id" uuid NULL, "appointment_id" uuid NULL, "code" text NOT NULL, "description" text NOT NULL, "amount_value" text NOT NULL, "amount_currency" text NOT NULL, "posted_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "invoices" ( "id" uuid NOT NULL PRIMARY KEY, "account_id" uuid NOT NULL, "status" text NOT NULL DEFAULT 'draft', "total_value" text NOT NULL, "total_currency" text NOT NULL, "charge_ids" jsonb NOT NULL DEFAULT ('[]'::jsonb), "finalized_at" timestamp with time zone NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "payments" ( "id" uuid NOT NULL PRIMARY KEY, "invoice_id" uuid NOT NULL, "amount_value" text NOT NULL, "amount_currency" text NOT NULL, "method" text NOT NULL, "reference" text NULL, "posted_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "consents" ( "id" uuid NOT NULL PRIMARY KEY, "patient_id" uuid NOT NULL, "consent_type" text NOT NULL, "status" text NOT NULL DEFAULT 'active', "granted_date" date NOT NULL, "expiry_date" date NULL, "revoked_date" date NULL, "purpose" text NULL, "method" text NULL, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL );

CREATE TABLE IF NOT EXISTS "audit_log" ( "id" uuid NOT NULL PRIMARY KEY, "entity_type" text NOT NULL, "entity_id" uuid NOT NULL, "action" text NOT NULL, "old_value" jsonb NULL, "new_value" jsonb NULL, "user_id" text NULL, "user_ip" text NULL, "user_agent" text NULL, "at" timestamp with time zone NOT NULL DEFAULT (now()) );

CREATE TABLE IF NOT EXISTS "outbox_events" ( "id" uuid NOT NULL PRIMARY KEY, "event_type" text NOT NULL, "payload" jsonb NOT NULL, "published" bool NOT NULL DEFAULT FALSE, "at" timestamp with time zone NOT NULL DEFAULT (now()) );

CREATE INDEX "idx_appointments_patient_start" ON "appointments" ("patient_id", "start_datetime");

CREATE INDEX "idx_appointments_practitioner_start" ON "appointments" ("practitioner_id", "start_datetime");

CREATE INDEX "idx_encounters_patient_status" ON "encounters" ("patient_id", "status");

CREATE INDEX "idx_waitlist_service_priority" ON "waitlist_entries" ("target_service", "priority", "created_at");

CREATE INDEX "idx_rtt_clock_events_pathway_at" ON "rtt_clock_events" ("pathway_id", "event_at");

CREATE INDEX IF NOT EXISTS idx_patients_active ON patients (active) WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_slots_schedule_start_free ON slots (schedule_id, start_datetime) WHERE status = 'free';

CREATE INDEX IF NOT EXISTS idx_bed_assignments_active ON bed_assignments (bed_id) WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_outbox_unpublished ON outbox_events (id) WHERE published = false;

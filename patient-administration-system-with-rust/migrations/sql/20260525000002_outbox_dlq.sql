-- Outbox dead-letter queue (v0.5.0).
--
-- Extends `outbox_events` with retry tracking (`retry_count`,
-- `last_attempted_at`, `last_error`) and adds the `outbox_dead_letters`
-- table. When a publish has failed `PAS_OUTBOX_MAX_RETRIES` times, the
-- dispatcher moves the row from `outbox_events` to `outbox_dead_letters`
-- so the hot path stays cheap and an operator can review / replay the
-- failure via the admin endpoints.

ALTER TABLE "outbox_events" ADD COLUMN "retry_count" integer NOT NULL DEFAULT 0;

ALTER TABLE "outbox_events" ADD COLUMN "last_attempted_at" timestamp with time zone NULL;

ALTER TABLE "outbox_events" ADD COLUMN "last_error" text NULL;

CREATE TABLE IF NOT EXISTS "outbox_dead_letters" ( "id" uuid NOT NULL PRIMARY KEY, "original_id" uuid NOT NULL, "event_type" text NOT NULL, "payload" jsonb NOT NULL, "created_at" timestamp with time zone NOT NULL, "dead_lettered_at" timestamp with time zone NOT NULL DEFAULT (now()), "retry_count" integer NOT NULL, "last_error" text NOT NULL );

CREATE INDEX "idx_outbox_dead_letters_at" ON "outbox_dead_letters" ("dead_lettered_at");

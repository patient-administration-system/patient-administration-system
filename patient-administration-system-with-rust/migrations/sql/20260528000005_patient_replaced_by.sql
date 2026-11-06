-- Patient `replaced_by` / merge tombstone (v0.11.0).
--
-- Adds a nullable `replaced_by UUID` column to `patients`. When the
-- sister MPI crate (or an operator) determines two patient rows refer
-- to the same person, one row becomes the *survivor* and the other
-- becomes a *tombstone* pointing at the survivor:
--
-- - `tombstone.replaced_by = survivor.id`
-- - `tombstone.active = false`
--
-- Tombstones are never hard-deleted — their `created_at` /
-- `updated_at` / sub-records remain so the audit trail can replay the
-- merge. Default list / search paths filter them out via
-- `replaced_by IS NULL`.

ALTER TABLE "patients" ADD COLUMN "replaced_by" uuid NULL;

CREATE INDEX IF NOT EXISTS idx_patients_replaced_by ON patients (replaced_by) WHERE replaced_by IS NOT NULL;

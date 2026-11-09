-- Suspected conditions come from a profile (WPAS-8, spec 20, FR-220, FR-221, NFR-26).
--
-- Migration 58 fixed the conditions and one country's cancer site list in CHECK constraints. Which conditions
-- a deployment tracks, their subtypes and their targets now come from the profile named by
-- PAS_SUSPICION_PROFILE, so those lists leave the database. What a suspicion was recorded under is copied onto
-- it (the profile, the target, whether a patient's cancel is held, the booking priority), so a later change of
-- profile never rewrites a wait that is already counting.

ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_condition_check;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_tumour_site_check;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_site_only_for_cancer;

ALTER TABLE suspected_conditions RENAME COLUMN tumour_site TO subtype;
ALTER TABLE suspected_conditions ADD COLUMN subtype_scheme text NULL;
ALTER TABLE suspected_conditions ADD COLUMN profile text NULL;
ALTER TABLE suspected_conditions ADD COLUMN target_days integer NULL;
ALTER TABLE suspected_conditions ADD COLUMN hold_patient_cancel boolean NOT NULL DEFAULT false;
ALTER TABLE suspected_conditions ADD COLUMN booking_priority text NULL;

-- Rows from migration 58 were recorded under its fixed rules: the Welsh cancer site list and target.
UPDATE suspected_conditions SET profile = 'wales' WHERE profile IS NULL;
UPDATE suspected_conditions SET subtype_scheme = 'wales-primary-cancer-site' WHERE subtype IS NOT NULL;
UPDATE suspected_conditions SET target_days = 62, hold_patient_cancel = true, booking_priority = 'urgent' WHERE condition = 'cancer';
UPDATE suspected_conditions SET condition = 'dementia' WHERE condition = 'alzheimers_disease';
ALTER TABLE suspected_conditions ALTER COLUMN profile SET NOT NULL;

ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_condition_shape CHECK (condition ~ '^[a-z0-9_]{1,64}$');
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_profile_shape CHECK (profile ~ '^[a-z0-9_-]{1,64}$');
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_subtype_whole CHECK ((subtype IS NULL) = (subtype_scheme IS NULL));
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_subtype_shape CHECK (subtype IS NULL OR (subtype ~ '^[A-Za-z0-9._-]{1,16}$' AND subtype_scheme ~ '^[a-z0-9_-]{1,64}$'));
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_target_days CHECK (target_days IS NULL OR target_days BETWEEN 1 AND 3650);
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_booking_priority_check CHECK (booking_priority IS NULL OR booking_priority IN ('routine','urgent','two_week_wait','emergency'));

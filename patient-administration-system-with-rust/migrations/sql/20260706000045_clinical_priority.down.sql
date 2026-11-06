DROP INDEX IF EXISTS idx_waitlist_priority_review;

ALTER TABLE waitlist_entries DROP CONSTRAINT IF EXISTS waitlist_entries_clinical_priority_chk;

ALTER TABLE waitlist_entries DROP COLUMN IF EXISTS clinical_priority_review_by, DROP COLUMN IF EXISTS clinical_priority_set_by, DROP COLUMN IF EXISTS clinical_priority;

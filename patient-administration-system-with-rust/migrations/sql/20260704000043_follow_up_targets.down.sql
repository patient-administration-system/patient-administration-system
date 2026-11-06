DROP INDEX IF EXISTS idx_waitlist_follow_up_target;

ALTER TABLE waitlist_entries DROP CONSTRAINT IF EXISTS waitlist_entries_follow_up_target_chk;

ALTER TABLE waitlist_entries DROP COLUMN IF EXISTS follow_up_target_set_by, DROP COLUMN IF EXISTS follow_up_target;

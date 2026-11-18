-- Back to the list before TSC-2: a step it skipped is kept as skipped (late), not lost.
UPDATE reminder_steps SET outcome = 'skipped_late' WHERE outcome = 'skipped_not_booked';
ALTER TABLE reminder_steps DROP CONSTRAINT IF EXISTS reminder_steps_outcome_check;
ALTER TABLE reminder_steps ADD CONSTRAINT reminder_steps_outcome_check CHECK (outcome IN ('sent','failed','skipped_replied','skipped_late','skipped_no_channel','skipped_opted_out','capped','call_task'));

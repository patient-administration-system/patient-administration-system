-- TSC-2: a ladder step whose appointment was cancelled or moved between the scan and the step is logged as
-- `skipped_not_booked` rather than sent.
ALTER TABLE reminder_steps DROP CONSTRAINT IF EXISTS reminder_steps_outcome_check;
ALTER TABLE reminder_steps ADD CONSTRAINT reminder_steps_outcome_check CHECK (outcome IN ('sent','failed','skipped_replied','skipped_late','skipped_no_channel','skipped_opted_out','capped','call_task','skipped_not_booked'));

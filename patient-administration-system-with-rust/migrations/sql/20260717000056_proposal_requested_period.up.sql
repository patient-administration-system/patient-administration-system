-- The window a `$find` asked for, kept on each proposal (NEW-10). The IHE Scheduling profile requires
-- `Appointment.requestedPeriod` on every appointment it returns; the window is what that period is.
-- Proposals made before this migration have none and answer with their own slot's time instead.

ALTER TABLE slot_proposals ADD COLUMN IF NOT EXISTS requested_start timestamptz NULL, ADD COLUMN IF NOT EXISTS requested_end timestamptz NULL;

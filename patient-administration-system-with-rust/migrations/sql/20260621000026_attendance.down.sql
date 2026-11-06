DROP INDEX IF EXISTS idx_appointments_in_facility;

ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_current_room_chk;

ALTER TABLE appointments DROP COLUMN IF EXISTS current_room, DROP COLUMN IF EXISTS completed_at, DROP COLUMN IF EXISTS arrived_at;

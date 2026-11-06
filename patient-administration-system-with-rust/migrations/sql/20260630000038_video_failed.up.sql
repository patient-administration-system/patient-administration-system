-- A video visit that fails to connect falls back to a phone call (CAP-14): `staff_call_tasks` learns the
-- reason `video_failed`, so someone phones the patient instead of the visit being lost.

ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check;

ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed','high_risk','video_failed'));

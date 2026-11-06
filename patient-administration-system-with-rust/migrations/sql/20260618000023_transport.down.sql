DROP TABLE IF EXISTS transport_bookings;

DROP TABLE IF EXISTS transport_consents;

DROP TABLE IF EXISTS transport_eligibility;

ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check;

ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ('silent','reschedule_request','urgent_pathway'));

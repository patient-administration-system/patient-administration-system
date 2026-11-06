DROP TABLE IF EXISTS noshow_reviews;

DROP TABLE IF EXISTS noshow_assessments;

ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check;

ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed'));

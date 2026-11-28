-- Undo PA-01. The bookings it marked keep their status as `failed` (their made-up window is gone, and calling
-- them booked again would bring back the harm), and new `no_provider` ones become `failed` too.

UPDATE transport_bookings SET status = 'failed', updated_at = now() WHERE status IN ('no_provider','unconfirmed');

DELETE FROM staff_call_tasks WHERE reason = 'transport_unconfirmed';

ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check;

ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed','high_risk','video_failed'));

DROP INDEX IF EXISTS uq_transport_active;

CREATE UNIQUE INDEX IF NOT EXISTS uq_transport_active ON transport_bookings (appointment_id) WHERE status IN ('requested','booked');

ALTER TABLE transport_bookings DROP CONSTRAINT IF EXISTS transport_bookings_status_check;

ALTER TABLE transport_bookings ADD CONSTRAINT transport_bookings_status_check CHECK (status IN ('requested','booked','cancelled','failed'));

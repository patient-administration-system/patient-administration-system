-- PA-01: a transport booking is "booked" only when a provider accepted it.
--
-- - `transport_bookings.status` learns `no_provider` (no transport provider is configured, so no ride was asked
--   for and staff must arrange it) and `unconfirmed` (a booking the stand-in client made up before this
--   change: it has a reference and a pick-up window, but no provider was ever asked).
-- - One open booking per appointment still: the partial unique index counts both, so a repeated confirmation
--   adds nothing.
-- - `staff_call_tasks` learns the reason `transport_unconfirmed`: each upcoming appointment with a made-up
--   booking gets one, so a person arranges the ride and tells the patient.
-- - The made-up bookings are marked `unconfirmed` and lose their made-up pick-up window; the made-up reference
--   is kept (`stub-...`) so a person can match it to what the patient was told.

ALTER TABLE transport_bookings DROP CONSTRAINT IF EXISTS transport_bookings_status_check;

ALTER TABLE transport_bookings ADD CONSTRAINT transport_bookings_status_check CHECK (status IN ('requested','booked','cancelled','failed','no_provider','unconfirmed'));

DROP INDEX IF EXISTS uq_transport_active;

CREATE UNIQUE INDEX IF NOT EXISTS uq_transport_active ON transport_bookings (appointment_id) WHERE status IN ('requested','booked','no_provider','unconfirmed');

ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check;

ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed','high_risk','video_failed','transport_unconfirmed'));

INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at)
SELECT gen_random_uuid(), b.appointment_id, b.person_id, 'transport_unconfirmed', now()
  FROM transport_bookings b JOIN appointments a ON a.id = b.appointment_id
 WHERE b.status = 'booked' AND b.provider_ref LIKE 'stub-%'
   AND a.status = 'booked' AND a.start_datetime > now()
ON CONFLICT (appointment_id, reason) DO NOTHING;

UPDATE transport_bookings
   SET status = 'unconfirmed', window_start = NULL, window_end = NULL,
       last_error = 'made up by the stand-in transport client: no provider was asked', updated_at = now()
 WHERE status = 'booked' AND provider_ref LIKE 'stub-%';

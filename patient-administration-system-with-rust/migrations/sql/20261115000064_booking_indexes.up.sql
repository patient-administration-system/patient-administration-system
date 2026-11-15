-- Indexes for the booking path, found by the first load test (PERF-1, NFR-29).
--
-- Every booking checks that the room, clinician or equipment is not busy in another schedule at the same
-- time (`check_owner_free`, CORE-3). With no index on `appointments.slot_id` the planner answered "does an
-- appointment hold this slot" by hashing every live appointment, about 95 ms on 78,000 rows, on each booking;
-- with no index on the schedule owner it read every schedule twice. With these three it takes under 0.1 ms.
-- The same `slot_id` lookup is in the group-session count, the IHE hold and cancelling.

CREATE INDEX IF NOT EXISTS idx_appointments_slot ON appointments (slot_id) WHERE slot_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_schedules_owner ON schedules (owner_kind, owner_id);

-- The existing slot index covers free slots only; the clash check reads busy ones too.
CREATE INDEX IF NOT EXISTS idx_slots_schedule_start ON slots (schedule_id, start_datetime);

ANALYZE appointments;
ANALYZE schedules;
ANALYZE slots;

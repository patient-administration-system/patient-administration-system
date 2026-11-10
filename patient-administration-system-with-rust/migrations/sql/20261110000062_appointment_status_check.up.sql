-- appointments.status had no CHECK constraint (found by QUAL-28), though every other state column has one, so a
-- word the code does not know could be stored and then fail every read of that appointment. The values are
-- `AppointmentStatus` in `src/models/appointment.rs`; the drift test in `tests/qual28_state_lists_test.rs` keeps the
-- two the same. Added NOT VALID and then validated, so the check of existing rows does not block writes.
ALTER TABLE appointments ADD CONSTRAINT appointments_status_chk
    CHECK (status IN ('proposed','booked','arrived','fulfilled','cancelled','no_show')) NOT VALID;
ALTER TABLE appointments VALIDATE CONSTRAINT appointments_status_chk;

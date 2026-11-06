-- PILOT-2: widen the barrier codes. A patient who cannot come says why in their own terms; five more
-- reasons are now recognised (anxiety or fear, cost, no longer needed, could not cancel or did not
-- know how, and a communication or access need). The set is a CHECK so no route can store a value
-- the application does not understand.

ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_barrier_code_chk;

ALTER TABLE appointments ADD CONSTRAINT appointments_barrier_code_chk CHECK (barrier_code IS NULL OR barrier_code IN ('cannot_travel','caring','work','feel_better','unwell','forgot','other','anxiety','cost','no_longer_needed','could_not_cancel','access_need'));

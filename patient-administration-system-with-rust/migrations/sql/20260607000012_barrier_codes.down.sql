UPDATE appointments SET barrier_code = 'other' WHERE barrier_code IN ('anxiety','cost','no_longer_needed','could_not_cancel','access_need');

ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_barrier_code_chk;

ALTER TABLE appointments ADD CONSTRAINT appointments_barrier_code_chk CHECK (barrier_code IS NULL OR barrier_code IN ('cannot_travel','caring','work','feel_better','unwell','forgot','other'));

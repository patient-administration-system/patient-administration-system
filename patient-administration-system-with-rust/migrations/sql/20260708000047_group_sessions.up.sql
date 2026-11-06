-- Group consultations (NEW-18): a slot can declare how many patients it takes. 1 is a one-to-one
-- appointment, as every slot was before; more is a group session (a virtual group clinic, an education
-- session). The capacity is declared before anyone books: it is not overbooking.

ALTER TABLE slots ADD COLUMN IF NOT EXISTS capacity integer NOT NULL DEFAULT 1;

ALTER TABLE slots ADD CONSTRAINT slots_capacity_chk CHECK (capacity BETWEEN 1 AND 50);

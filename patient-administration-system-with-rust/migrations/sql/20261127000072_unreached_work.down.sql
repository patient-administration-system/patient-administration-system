-- Undo TSC-8: the unreached items and the phone outcomes recorded on them go.

DELETE FROM work_items WHERE kind = 'unreached';

ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_kind_check;

ALTER TABLE work_items ADD CONSTRAINT work_items_kind_check CHECK (kind IN ('call','follow_up','validation','needs_review','tolerance_alert'));

DROP TABLE IF EXISTS unreached_call_outcomes;

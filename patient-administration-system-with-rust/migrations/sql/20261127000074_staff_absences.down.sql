-- Undo TSC-10. Slots an active absence blocked are freed first, so none stays blocked with nothing to say why.

UPDATE slots SET status = 'free', updated_at = now()
 WHERE status = 'blocked_out'
   AND id IN (SELECT b.slot_id FROM absence_blocked_slots b JOIN staff_absences s ON s.id = b.absence_id
               WHERE s.withdrawn_at IS NULL);

DELETE FROM work_items WHERE kind = 'absence_rebook';

UPDATE work_views SET kind = NULL WHERE kind = 'absence_rebook';

ALTER TABLE work_views DROP CONSTRAINT IF EXISTS work_views_kind_check;

ALTER TABLE work_views ADD CONSTRAINT work_views_kind_check CHECK (kind IS NULL OR kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached'));

ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_kind_check;

ALTER TABLE work_items ADD CONSTRAINT work_items_kind_check CHECK (kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached'));

DROP TABLE IF EXISTS absence_appointments;

DROP TABLE IF EXISTS absence_blocked_slots;

DROP TABLE IF EXISTS staff_absences;

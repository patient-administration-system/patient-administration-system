ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_booking_priority_check;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_target_days;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_subtype_shape;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_subtype_whole;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_profile_shape;
ALTER TABLE suspected_conditions DROP CONSTRAINT IF EXISTS suspected_conditions_condition_shape;
ALTER TABLE suspected_conditions DROP COLUMN IF EXISTS booking_priority;
ALTER TABLE suspected_conditions DROP COLUMN IF EXISTS hold_patient_cancel;
ALTER TABLE suspected_conditions DROP COLUMN IF EXISTS target_days;
ALTER TABLE suspected_conditions DROP COLUMN IF EXISTS profile;
ALTER TABLE suspected_conditions DROP COLUMN IF EXISTS subtype_scheme;
ALTER TABLE suspected_conditions RENAME COLUMN subtype TO tumour_site;
-- Rows recorded under another profile may not fit migration 58's lists, so these are not re-checked.
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_condition_check CHECK (condition IN ('cancer','obstructive_sleep_apnoea','diabetes','cardiovascular_disease','parkinsons_disease','alzheimers_disease')) NOT VALID;
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_tumour_site_check CHECK (tumour_site IS NULL OR tumour_site IN ('01','02','03','04','05','06','07','08','09','10','11','12','13','14','98')) NOT VALID;
ALTER TABLE suspected_conditions ADD CONSTRAINT suspected_conditions_site_only_for_cancer CHECK ((condition = 'cancer') = (tumour_site IS NOT NULL)) NOT VALID;

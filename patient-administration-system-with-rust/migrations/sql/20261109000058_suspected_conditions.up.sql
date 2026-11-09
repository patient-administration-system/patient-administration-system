-- Suspected conditions on a referral (WPAS-1, spec 20, FR-207 to FR-216).
--
-- A referral is often made on suspicion of a condition, before a diagnosis: cancer above all, but also sleep
-- apnoea, diabetes, cardiovascular disease, Parkinson's and Alzheimer's. The wait that matters starts at the
-- point of suspicion and never pauses; it stops once, for one reason. The PAS records what a named clinician
-- decided; it decides nothing.

CREATE TABLE IF NOT EXISTS suspected_conditions (
    id uuid PRIMARY KEY,
    referral_id uuid NOT NULL REFERENCES referrals (id),
    person_id uuid NOT NULL,
    condition text NOT NULL,
    tumour_site text NULL,
    unique_pathway_identifier text NULL,
    suspected_on date NOT NULL,
    suspected_by text NOT NULL,
    recorded_by text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    stop_reason text NULL,
    stopped_on date NULL,
    stopped_by text NULL,
    stopped_at timestamptz NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT suspected_conditions_condition_check CHECK (condition IN ('cancer','obstructive_sleep_apnoea','diabetes','cardiovascular_disease','parkinsons_disease','alzheimers_disease')),
    -- The Welsh "Primary Cancer Site Description" list (NHS Wales Data Dictionary, valid from 2021-01-01).
    CONSTRAINT suspected_conditions_tumour_site_check CHECK (tumour_site IS NULL OR tumour_site IN ('01','02','03','04','05','06','07','08','09','10','11','12','13','14','98')),
    CONSTRAINT suspected_conditions_site_only_for_cancer CHECK ((condition = 'cancer') = (tumour_site IS NOT NULL)),
    CONSTRAINT suspected_conditions_upi_length CHECK (unique_pathway_identifier IS NULL OR char_length(unique_pathway_identifier) BETWEEN 1 AND 24),
    CONSTRAINT suspected_conditions_stop_reason_check CHECK (stop_reason IS NULL OR stop_reason IN ('treatment_started','downgraded','died','entered_in_error')),
    -- A clock stops once: the reason, the date and who recorded it come together or not at all.
    CONSTRAINT suspected_conditions_stop_whole CHECK ((stop_reason IS NULL) = (stopped_on IS NULL) AND (stop_reason IS NULL) = (stopped_by IS NULL) AND (stop_reason IS NULL) = (stopped_at IS NULL)),
    CONSTRAINT suspected_conditions_stop_after_start CHECK (stopped_on IS NULL OR stopped_on >= suspected_on)
);

CREATE INDEX IF NOT EXISTS suspected_conditions_referral ON suspected_conditions (referral_id);
CREATE INDEX IF NOT EXISTS suspected_conditions_person ON suspected_conditions (person_id);
CREATE INDEX IF NOT EXISTS suspected_conditions_open ON suspected_conditions (suspected_on, id) WHERE stop_reason IS NULL;
CREATE INDEX IF NOT EXISTS suspected_conditions_stopped ON suspected_conditions (stopped_on) WHERE stop_reason IS NOT NULL;
-- One open suspicion of a condition per referral; a second would double-count the same wait.
CREATE UNIQUE INDEX IF NOT EXISTS suspected_conditions_one_open ON suspected_conditions (referral_id, condition) WHERE stop_reason IS NULL;

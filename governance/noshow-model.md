# The no-show probability: model card, gate and review (CAP-12, CAS-5)

**Status: built, switched off.** Nothing is scored, shown or listed until the gate below is passed by named people. This document is for those people.

## What it is

A **transparent weighted sum**, not a trained model. For each booked appointment still to come it estimates how likely the patient is to miss it, from:

| Factor | What it uses | Notes |
|---|---|---|
| Base rate | a configured starting rate (default 10%) | what is assumed with no information |
| Past visits | how many of the patient's *completed* past visits were missed | shrunk towards the base rate for small samples, so "1 of 2" is not read as 50%; the sample size is shown |
| Lead time | days from booking to the visit, to a cap of 90 | a small linear adjustment |
| Age band | the coarse band from `person_demographics` | may only raise the figure |
| Day of week, part of day | clinic-local | **zero by default**: the factors most open to steering unreliable slots to particular groups |

Every number is stored with its factors, so the front desk sees *why*. Weights are configuration (`PAS_NOSHOW_WEIGHTS`, JSON); thresholds are configuration (`PAS_NOSHOW_MEDIUM` 0.40, `PAS_NOSHOW_HIGH` 0.70). A band is derived when read (above the threshold, not at it), so changing a threshold changes the bands without recomputing. `GET /api/noshow/model-card` returns this description with the live weights.

## What it may and may not do (NFR-11)

It **only adds support**: an appointment above the high threshold gets a task on the staff priority list (`GET /api/noshow/priority-list`), a person rings the patient, and records an outcome (reached, no answer, declined help, wrong number), which clears it.

It **never** removes, reorders away or withholds a slot, never overbooks, never delays a contact, and acts on nothing by itself (UK GDPR Article 22: staff decide). Enforced by:

- the code: nothing in booking, rebooking, standby or offers reads it, and a test fails if the scoring module's source names those tables or changes an appointment;
- a behaviour test: rebooking options and standby candidates are identical, in content and order, with and without scores (`cas5_noshow_test.rs`);
- the schema guard (`cap10_support_test.rs`), which lists the one allowed column by name and reason.

## The gate

It is on only when **all** of these hold:

1. `PAS_NOSHOW_SCORE=true`;
2. a named person has **approved a fairness review** of this model version, in the last 180 days;
3. a named person has **approved a clinical safety review** of this model version, in the last 365 days;
4. nobody has since **rejected** either (a rejection switches it off at once).

`GET /api/noshow/status` says whether it is on and, if not, exactly why. Reviews are recorded with `POST /api/noshow/reviews` (`kind`, `reviewer`, `outcome`, `notes`); an approval needs notes of at least 40 characters saying what was looked at. Each is audited.

## The evidence for the fairness review

`GET /api/noshow/fairness-audit?days=365` backtests the model on appointments that already have an outcome, scoring each as the model would have with only what was known at the time, and splits the result by **deprivation quintile and language**: how many, mean predicted rate, observed no-show rate, the gap between them, and the share placed in the high band. Cells under `k` (10 or more) are withheld, with the same rules as the analytics feed. It works while the score is off, so the evidence exists before the approval.

What a reviewer should look for: a group with a much larger share in the high band than its observed rate supports; a large calibration gap of either sign in a group; a group where the model under-predicts or over-predicts consistently. Any of these is a reason to **reject, or change the weights and review again** (a changed weight set is a new decision, and the version string should be bumped in code with it).

## What this code cannot decide

- **Whether it is a medical device.** A risk score re-opens the question (DTAC C1.1.1), and the clinical safety case (DTAC-1, DCB0129). A person must decide both. The clinical safety review above is where that is recorded, not where it is decided.
- **Whether the model is any good here.** It is built from rules, not trained on this population. Its numbers are estimates to prompt a conversation, not predictions to rely on. The audit shows how it would have done; a few months of real outcomes (PILOT-10) should be read before approval.
- **Whether the support on offer is good support.** A phone call from someone who can help (transport, a different time, a worry) is the point; a call that only reminds is not.

## Not built

Distance from the clinic (needs a postcode-district band nobody supplies), clinic type, weather or travel forecasts, and a trained model. The daily pass is hourly and idempotent within a day.

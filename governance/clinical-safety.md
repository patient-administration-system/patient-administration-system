# Clinical safety: hazard log (DRAFT, needs a Clinical Safety Officer)

**This is not a DCB0129 clinical safety case.** There is no Clinical Safety Officer, no agreed risk-scoring scheme and no clinician review. It lists hazards found in the design and code so a CSO can start from them. Likelihood and severity are left blank on purpose: a clinician assigns them.

Scope: an administrative system whose events (admissions, bed status, appointments) feed clinical systems, so an administrative error can become a care error.

| ID | Hazard | Cause | Existing control | Gap / action | L | S |
|---|---|---|---|---|---|---|
| H-01 | Patient is told to attend the wrong time or place | Template or appointment edited wrongly; wrong time zone (all times are UTC) | Appointment state machine; audit; template syntax check on save (not yet tested) | Show local time; approval step for template edits; test FR-7 | | |
| H-02 | Reminder sent for a cancelled or completed appointment | Race between cancel and the reminder scan | Scan selects `booked` only | Re-check status at send time; test | | |
| H-03 | Reminder sent twice, or never | Retries, crash between send and record | A rendered letter is the sent marker; tested for double scan | Crash-window not tested; no delivery receipts (NOT-4) | | |
| H-04 | Message reaches the wrong person | Stale or shared number | None | Contact verification (CAP-4); portal link instead of detail (NFR-13) | | |
| H-05 | Urgent patient misses care and nobody follows up | A miss is only recorded | None | Clinical-urgency flag and mandatory human follow-up (CAP-7) | | |
| H-06 | Patient discharged from a list automatically after a miss | Future automation | None exists | Rule: a miss triggers contact, never discharge (CAP-7) | | |
| H-07 | Patient placed in the wrong bed or bed double-booked | Concurrent admit/transfer | Transactions; bed status | Concurrency test not written | | |
| H-08 | A cancellation by reply is applied to the wrong appointment or patient | Ambiguous reply, forged request | None yet (not built) | Authenticated, idempotent write-back (CAP-2) | | |
| H-09 | Worker merge leaves records pointing at a retired staff id | Missed table in the merge | `worker-merged` webhook, tested for all tables after BUG-1 | Re-test when a table gains a worker column | | |
| H-10 | A support-only risk score removes help or reduces slots | The no-show probability (CAP-12) | NFR-11, now tested: rebooking and standby are identical with and without scores; the scoring module never names slot, offer or standby tables; a schema guard; the score is switched off unless a fairness and a clinical safety review are approved by named people (`governance/noshow-model.md`) | A CSO must decide whether it is a medical device and review the model; the weights are untested on real outcomes | | |
| H-13 | A score falls harder on one group (deprivation, language, age), so that group is flagged more or offered less | Proxies for protected characteristics in the factors | Age may only raise the figure; day and time are zero by default; the fairness audit by quintile and language; a rejection switches it off at once | Read the audit on real data before approval; re-review when weights change | | |
| H-11 | Event not delivered to a clinical system | Publisher failure | Outbox retries, dead-letter queue, replay | Alerting on dead letters not built | | |
| H-12 | Wrong person's data shown to a patient | Wrong `person_id` mapping on the account | Server-side scope; tested | Proofing process (DTAC-9) | | |
| H-14 | A patient who still needs care is removed from a waiting list by validation, or a removal goes unreported | Validation campaign (NEW-2): a patient who does not reply, cannot reply, or replies through someone else | Removal only by a named person after a letter and a wait; never for "still needed", "unsure" or an urgent service; reversible for 90 days with the clock kept; reported per service apart from treatment (`tests/new2_validation_test.rs`) | Reach people with accessible-communication needs their way before "no reply"; letter wording; who may decide; clinical safety officer review of the process (`governance/waiting-list-validation.md`) | | |

How this log lines up with the ISO 14971 process, and what that process would still need: [`risk-management-iso-14971.md`](risk-management-iso-14971.md).

Who may close a hazard, and on what evidence: [`spec/hazard/index.md`](../spec/hazard/index.md). Until a CSO is named, none can be closed.

## Process still required
Name a CSO; agree a risk-scoring scheme; review and score the above; add hazards from clinicians; produce the Clinical Safety Case Report; keep the log under change control.

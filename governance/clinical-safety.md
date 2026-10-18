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
| H-10 | A support-only risk score removes help or reduces slots | Future scoring | NFR-11 (rule, not yet testable) | Test written with the score (CAP-10) | | |
| H-11 | Event not delivered to a clinical system | Publisher failure | Outbox retries, dead-letter queue, replay | Alerting on dead letters not built | | |
| H-12 | Wrong person's data shown to a patient | Wrong `person_id` mapping on the account | Server-side scope; tested | Proofing process (DTAC-9) | | |

## Process still required
Name a CSO; agree a risk-scoring scheme; review and score the above; add hazards from clinicians; produce the Clinical Safety Case Report; keep the log under change control.

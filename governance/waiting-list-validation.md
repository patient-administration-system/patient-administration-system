# Waiting-list validation

DRAFT, written by the developers, not reviewed by a clinical safety officer (DTAC-1) or an information governance lead.

Validation asks long-waiting patients whether they still need the care, so that the list is true. Done badly it removes people who still need care, hides the removals, and falls hardest on people who cannot reply (no mobile, no fixed address, language or access needs). The PAS builds the process so that none of that can happen by accident. What it cannot do is decide who is safe to remove: that stays with named people.

## What the software enforces (`list_validation`, tested)

- A campaign is started by a named person and takes only long waiters of one service. **A service listed for human follow-up (urgent and cancer pathways) is never taken, and a removal on one is refused.**
- Nothing contacts the patient from the PAS. A person asks (phone, text, in person) and records it; until a carrier exists (NOT-3) this is the only route.
- **A patient is removed only by a named person's decision**, and only when the patient said the care is no longer needed, or was asked, did not answer, was sent **a letter** (posted by a named person, only after the reply date) and the wait for a reply to the letter ended. A patient who said they still need it, or are unsure, is never removed.
- A removal removes the waiting entry and stops the RTT clock with the reason `validation_removal`, audited. It is **reversible for 90 days**: the entry returns and the clock carries on as if nothing happened.
- Removals, reinstatements and keeps are **reported per service apart from treatment** (`GET /api/reports/validation-removals`, no identifiers), because unreported removals are what regulators criticise.

## What people must decide

- Who may decide a removal (the API checks that a name is given, not who they are: DTAC-9 and per-user authentication are the controls).
- The reply period and the letter wait (defaults 14 and 14 days). Long enough for someone on holiday, in hospital or waiting for post.
- **How the letter and any text read, in Welsh and English and in accessible formats** (`governance/messaging-rules.md`); a patient with a recorded accessible-communication need must be reached in that way before anyone is treated as "no reply".
- Whether to ask the referrer before removing someone (the PAS does not do this yet).
- What a "no longer needed" answer by a carer or relative means.
- Reporting: where the validation figures are published and who reads them.

## Not built

A patient-facing reply page or text keyword (needs NOT-3 and translated wording), automatic letters, a letter to the referrer, a staff page for the work lists, and the link to the national waiting-list reporting.

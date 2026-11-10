# Hazards: who may remove one, and on what evidence

**Status: a rule for the hazard log, written 2026-10-28. Checked by `scripts/check-hazard-log.py` since 2026-11-10 (SAFE-2, SAFE-3); not yet routed to a CSO by CODEOWNERS, because none is named.** The hazard log is [`governance/clinical-safety.md`](../../governance/clinical-safety.md) (a markdown table, DRAFT). No Clinical Safety Officer is named yet, so today nobody can close a hazard under this rule; every hazard stays open. That is the safe failure.

## The rule

1. **Only the Clinical Safety Officer (CSO) can remove a hazard from the active log.** Nobody else may, including the maintainer, the author of the control, a reviewer or an administrator.
2. **The CSO must record evidence that the hazard has been removed.** A closure with no evidence is invalid and the hazard stays open.
3. **Removal means closing, not deleting.** A closed hazard stays in the log with its history. Its identifier is never reused. What leaves the *active* list is its open status.

## Assessment and changes from the first wording

The request said "chief safety officer" and "remove". Both are tightened here; the reasoning is below so the CSO can overrule it.

- **The role is the Clinical Safety Officer.** DCB0129 (manufacturers) and DCB0160 (deploying organisations) each require one, and the officer must be a suitably qualified and experienced clinician. There is no "chief" role in either. The role needs a named person and a named deputy; see "Absence" below. (These standards were read through secondary summaries, not the NHS England text; check the wording against [DCB0129](https://digital.nhs.uk/data-and-information/information-standards/governance/latest-activity/standards-and-collections/dcb0129-clinical-risk-management-its-application-in-the-manufacture-of-health-it-systems) before relying on it. See also [`risk-management-iso-14971.md`](../../governance/risk-management-iso-14971.md).)
- **"Remove" must not be deletion.** A log that can lose rows cannot reconstruct what was known at the time of an incident. Secondary sources on DCB0129 say the log should be versioned so its state at any date can be rebuilt, and that each entry carries a status (open, mitigated, accepted, closed). Keep rows, change status.
- **"Removed" is several different things.** A hazard is not always "gone". Each needs different evidence, and only the first two take it off the active list:

  | Closure reason | What it means | Evidence the CSO must record |
  |---|---|---|
  | `eliminated` | The cause no longer exists (the feature was removed, the data is no longer held) | The change that removed it (commit and release) and a test or check that fails if it returns |
  | `controlled` | A control is in place and the residual risk is acceptable | The control; the test that exercises it, by name and result; the re-scored likelihood and severity; the CSO's acceptability decision |
  | `accepted` | A risk remains and the CSO accepts it | The residual score, the reasoning, and who else was consulted. It stays visible on the log as accepted, not closed |
  | `not_applicable` | The hazard was recorded in error or the scenario cannot occur | The reasoning, with a second reader (below) |

- **The CSO should not be the only person who looked.** The rule makes the CSO the only person who *may* close. It should not make the CSO the only person who *checks*. Closing a hazard needs two things from two people: the owner of the control submits the evidence, and the CSO decides. The same pattern is already used in this system (a template version, a no-show review and a validation decision each need an approver different from the proposer). A CSO who is also the control's author cannot close that hazard alone; a second clinician reviews it.
- **Evidence must be checkable later.** A link that can change is not evidence. Record a commit hash, the test name and its run (the CI run identifier), and the date. For documents, a content hash. The evidence is attached to the closure and never edited; a correction is a new entry.
- **A closed hazard reopens when its ground moves.** Any change to code, data or process named in the hazard's controls reopens it for review (CODEOWNERS can route such changes to the CSO). New clinical functions need a new hazard assessment before release.
- **Add up the residual risks.** Many individually acceptable risks can still be unacceptable together. The CSO's closing decision for the whole log (the Clinical Safety Case Report) considers the total, not only each row.
- **Absence.** If the CSO is away or leaves, closure stops, not the work: hazards stay open, and a named deputy CSO acts. A system with no CSO cannot close anything, which is what is true today.

## What a closure record holds

| Field | Notes |
|---|---|
| Hazard ID | Never reused |
| Closure reason | One of the four above |
| Residual likelihood and severity | Set by a clinician, never defaulted |
| Evidence | One or more items, each with a kind, a reference that does not change, and a date |
| Closed by | The CSO, by name and role |
| Evidence submitted by | A different person where the CSO wrote the control |
| Closed at | Date and time, London time |
| Version of the log | So the log can be rebuilt as it stood |
| Reopened at / why | If the hazard returns to the active list |

## How this applies today, and what is not built

- Today the log is a markdown table in git with a `Status` column, and closures are lines in [`governance/hazard-closures.csv`](../../governance/hazard-closures.csv). `scripts/check-hazard-log.py`, run by CI, refuses a status of `closed` or `accepted` with no record to match it; a record not by the CSO or deputy named in the log; evidence submitted by the person closing; a closure with no evidence, or with evidence whose reference can change (only a full commit hash, a test with its CI run, or a document with its SHA-256 is accepted); `controlled` or `accepted` with no residual score; a deleted hazard row; and an edited closure line (the file is append-only). While nobody is named, every closure is refused. It is still possible to change the rule by changing the script: CODEOWNERS on the script, the log and the closure file, routed to the CSO, is the remaining step (SAFE-2).
- Stricter than the rule above: the evidence is always submitted by someone other than the person closing, not only where the CSO wrote the control, because the check cannot tell who wrote a control. See `tasks.md` SAFE-1 to SAFE-4 for the way to enforce it (a CODEOWNERS entry, a check that no hazard identifier disappears, a closure record, and a CSO-only route).
- The rule does not make the system safe by itself. It controls who may say a hazard is gone and what they must show. Whether the hazard is gone is the CSO's clinical judgement.

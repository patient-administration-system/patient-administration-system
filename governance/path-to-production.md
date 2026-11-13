# Path to production

**Production** here means one thing: a health organisation runs this system for a live service, holding real patients' details, where a mistake can reach a person. Being built, tested and published is not production. Neither is a demonstration on made-up data.

To get there, the system needs more than code. It needs a named clinician accountable for its safety, a legal basis for the data it holds, outside security assurance, messages that really arrive, and an organisation that runs it and answers for it. Most of what is left is that kind of work.

The website shows this list with a slider per item: <https://patient-administration-system.github.io/path-to-production/>. Keep the two in step (the website's figures are in `src/lib/readiness.ts` there).

## How to read the estimates

Each figure is a judgement, not a measurement: how much of the work that item needs is done, from 0% (not started) to 100% (done, with evidence). Last reviewed **2026-11-13**. Change a figure only with a reason in the commit message. Raise one to 100% only with the evidence the item names.

An item that needs a person or an outside party stays low however much code surrounds it, because the code cannot do that part.

## Safety and assurance

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Clinical Safety Officer and DCB0129 safety case | 15% | Hazard log drafted ([`clinical-safety.md`](clinical-safety.md), 21 hazards, all open); closure rules checked in CI (SAFE-2, SAFE-3) | No Clinical Safety Officer named (SAFE-1); hazards not scored; no Clinical Safety Case Report (DTAC-1) |
| Data protection impact assessment signed off | 40% | Draft DPIA, data flow, record of processing and retention schedule ([`dpia.md`](dpia.md)) | No review by a Data Protection Officer; the controller's decisions are open (DTAC-2) |
| External penetration test | 0% | Security headers, CSRF and access rules are tested by the project itself | No external test has been done (DTAC-10) |
| Cyber Essentials and the supplier Cyber Security Charter | 0% | | Neither has been sought (DTAC-10) |
| Data Security and Protection Toolkit (deploying organisation) | 0% | | Belongs to the organisation that runs the system; there is none yet |

## Identity and access

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Multi-factor sign-in for staff | 85% | The shipped identity realm requires a one-time code for staff and administrators ([`mfa-plan.md`](mfa-plan.md), DTAC-5) | The deploying organisation's own identity provider must enforce the same |
| Per-user authentication at the API, with a trustworthy audit user | 70% | Signed per-user tokens, verified offline; the audit records the verified user (DTAC-4) | Opt-in (`PAS_REQUIRE_AUTH`), not the default; no production secret store for the signing key; no per-person tokens at the API |
| Sign-in for the operator front-end | 0% | | The operator dashboard has no sign-in; needs a decision (AXA-3) |
| Proof of a patient's identity before an account is linked | 60% | A named staff member records the proofing; patient pages refuse an unproofed account (DTAC-9) | No staff screen to record it (the API exists); NHS login is not integrated |

## Reliability

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Backups and a restore drill | 70% | A backup, restore and compare drill runs in CI on every push ([`../operations/backup-and-restore.md`](../operations/backup-and-restore.md)) | No live deployment has backups or has run the drill |
| Alerts on dead-letter events and failed sends | 40% | Prometheus metrics, and the runbook names what to alert on ([`../operations/runbook.md`](../operations/runbook.md)) | No alert is configured anywhere |
| Availability target, set and measured | 0% | | No target has been set |

## Messaging

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Consent, opt-out and quiet hours | 90% | Built and tested at the one point every message passes through (NOT-5) | Wording not reviewed with patients |
| Delivery receipts and retries | 80% | The system's side is built and tested (NOT-4) | Not tried with a real carrier |
| Real carriers (email, SMS, WhatsApp, national channels) | 10% | One interface every carrier would sit behind (`MessageSender`) | No carrier contract or sandbox send (NOT-3, NEW-14) |

## People and content

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Translations reviewed by first-language speakers | 10% | Drafts in every interface language; keys and placeholders checked by tests ([`translation-review.md`](translation-review.md)) | No translation reviewed; Welsh must be before patients see it |
| Accessibility checked by people, statement published | 30% | Automated axe and reflow checks in the browser tests; draft statement ([`accessibility-statement.md`](accessibility-statement.md)) | No checks by people; statement not published |
| Usability testing with patients and staff | 0% | | Not done (DTAC-11) |

## Supply chain

| Item | Estimate | Done | Not done |
|---|---:|---|---|
| Dependency licences and advisories checked | 100% | `cargo deny check` passes in CI; each exception justified in `deny.toml` | |
| CI required before a change reaches the main branch | 60% | Full CI runs on every push and must be green before a release | Branch protection is off, so nothing enforces it (QUAL-43) |
| Software bill of materials with each release | 100% | The release workflow makes CycloneDX SBOMs and attaches them (REL-1) | |

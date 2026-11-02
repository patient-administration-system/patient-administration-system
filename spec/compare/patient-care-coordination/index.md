# Comparison: Care Coordination and 360 Referrals (PatientCareCoordination)

Researched 2026-11-02 from <https://kevinmayfield.github.io/PatientCareCoordination/en/index.html> and five of its pages (Patient Administration, Closed Loop Referral 360X, Appointment/No-Show PCC-60/61, Patient Flags, BaRS). Nothing was run. The other pages (Reports, Virtual Wards, DCP, mCSD, PDQm, Transport, PRSB, the logical models) were **not read**. Ideas are in [`plan.md`](../../../plan.md) §15 and `tasks.md` (series `PCC`).

## What it is

An HL7 FHIR R4 implementation guide, version 0.1.0, by Kevin Mayfield, published as a local development build (draft 2026-11-02). It describes the **closed loop referral**: a clinician refers a patient, the recipient accepts or declines, books, sees the patient and sends the outcome back, and the referrer knows what happened. It restates IHE 360X (HL7 v2.5.1 origin) for NHS England and assesses eRS, BaRS, Transfer of Care, MESH, GP Connect, IM1 and EPS against it. **It says of itself that it is a proof of concept and demonstration only, not authoritative, not endorsed by NHS England, IHE or HL7, and must not be used for implementation or clinical purposes.** No licence is stated on the page. It is England-only; there is nothing Welsh in it (the BaRS page says so itself).

So it is a **design reference and a vocabulary of interactions**, not software and not a standard. Treat every "To confirm" in it as unconfirmed.

## What is useful to a PAS

1. **A referral state machine, with the edges that go wrong.** `requested`, `accepted`, `declined`, `information_requested`, `redirected`, `deferred`, `appointment_booked`, `no_show`, `interim_report`, `cancel_requested`, `cancelled`, `completed`. Terminal: declined, redirected, cancelled, completed. The useful rules: a decline can arrive **after** acceptance; a cancel request can lose to an outcome that arrived first ("too late to cancel"); a no-show leads to a rebook **or** a decline; a deferred referral resumes as accepted; an unsupported optional transaction is not an error.
2. **One referral ID, assigned by the sender, in every message, both directions.** After a decline a new referral needs a new ID; a redirect ends the referral and the new one `replaces` it. Where eRS is used the UBRN is a second ID known to both sides.
3. **Appointment and no-show as notifications to the referrer** (PCC-60 and PCC-61): `SIU^S12` new, `S13` reschedule, `S15` cancel, `S26` no-show, with the appointment ID in SCH-2, the **referral ID in SCH-26** and the date in TQ1-7; FHIR `Appointment.basedOn` carries the referral. A no-show leaves the referral open.
4. **Identifier discipline.** Name the identifier's type (`NH`, `MR`, `AN` spell, `VN` visit, `UBRN`) and carry the assigning organisation (an ODS code in England) on every one. Keep the spell and the visit apart.
5. **Registration rules.** One encounter per continuous period in one setting; ward and bed moves are location history, not new encounters; **no `planned` encounters** (future activity is an Appointment); one level of hierarchy only.
6. **Flags as pointers.** A flag says "a plan exists, here is who owns it"; the detail stays with the owner. Four kinds: plan pointer, status, risk, need with content. A referrer should not copy safeguarding detail into a referral.
7. **Architecture advice from the BaRS page:** adopt an external standard behind an **anti-corruption layer**, keep the received bundle unchanged for audit, and **report** gaps rather than absorbing them into the core model.

## Against this repository

| Topic | PatientCareCoordination | This repository |
|---|---|---|
| Referral states | 12 states, 4 terminal | `received`, `waitlisted`, `booked`, `closed`, `returned_with_advice`, `redirected` (`src/referrals/mod.rs`); a triaged referral cannot be triaged again; a redirect creates the new referral in the same transaction (FR-167, FR-168) |
| Referral ID | Sender's ID in every message; UBRN second | `(source, external_ref)` is one referral, receiving is idempotent; `source` is `gp`, `e-rs` or `internal`; an e-RS referral must carry a reference |
| Booked appointment linked to the referral | `Appointment.basedOn`; SCH-26 | `appointments.referral_id` column exists |
| Appointment events to others | SIU S12, S13, S15, S26 | The HL7 v2 publisher already sends S12, S13, S14, S15 and S26 (`src/streaming/hl7v2_publisher.rs`); `build_sch` fills SCH-1, -2, -7, -11 and -25 and **leaves SCH-26 empty** |
| Reply to the referrer | PCC-56/57/58/59/60/61 with accept, decline, outcome, cancel | Receive and triage only; **nothing tells the referrer** that the referral was booked, triaged, redirected, closed or missed |
| Cancelled-by-referrer | `cancel_requested` then `cancelled`, or lost to the outcome | Not found in the referral module *(inferred from a code search)* |
| More information needed | `information_requested` (NHS addition, not in 360X) | `returned_with_advice` is the nearest; no request-more-information state |
| On hold | `deferred` (provisional) | Not found |
| Identifier types and assigners | `NH`, `MR`, `AN`, `VN`; ODS assigner | Identity is delegated to person-service; PAS stores opaque UUIDs. Encounter and appointment identifiers sent outward carry no assigner *(inferred)* |
| Encounters | no `planned`; one level | `pre_admit` creates a `Planned` inpatient encounter (`src/adt/admission.rs`) |
| Flags | `Flag` plus a summary `CarePlan`, and the R4 link extension | `Flag` search is mapped to R5 for reasonable adjustments (NEW-6, QUAL-39); no plan pointer |
| FHIR version | R4 | R5 primary, with R4 for `AuditEvent` and the IHE scheduling operations |
| Standards stance | England (eRS, BaRS, MESH, GP Connect) | Worldwide, no home country (`spec/01`); Wales is the first deployment |

## Where this repository is ahead

Tolerances, fairness measurement, the safety case, DTAC, accessibility, ten languages, a tested outbox with a dead-letter state, read auditing, idempotent receipt of referrals, the IHE `$find` and `$hold` operations, and checks that cannot silently skip. The guide has no software, no tests and no deployment.

## Assessment

**Maturity.** A draft, one author, a build from today, with its own disclaimer. Many of its statements carry "To confirm". It cannot be cited as the authority for anything; the real authorities are IHE 360X, the NHS England BaRS and eRS documents, and, for Wales, the Digital Health and Care Wales specifications (not read here).

**Fit.** Its scope is **the referral conversation between two organisations**. The PAS is the **recipient-side administrative system** (receive, triage, wait list, book, attend, close). It is the one that should answer the loop, and today it does not: that is the most useful finding. It is administrative, so the clinical halves of the guide (the Interim Consultation Note, the Referral Outcome as a clinical document, the HL7 Europe Discharge Report, Dynamic Care Planning, virtual-ward care plans) are **out of scope** (`spec/02-scope.md`), and the PAS may only carry such a document as an opaque attachment if a clinical system sends one.

**Risks.**
- Building to an unendorsed proof of concept. Mitigation: take the *state machine and the rules*, which are small and checkable against 360X itself, and verify each against IHE 360X before building.
- Cancelling a referral the recipient has already acted on. The "too late to cancel" rule exists to stop this.
- A referrer who is never told. Today a returned or redirected referral is known only inside the PAS.
- A new outbound message is an outward action with a recipient. Needs a named destination and an agreement (not a decision for the code).
- England-shaped codes (ODS, NHS number, UBRN) must not be hard-wired; the worldwide stance (QUAL-40) applies.

**Licence.** None stated on the page; no code or text was copied. Re-derive anything from IHE 360X or the NHS England sources directly.

## Advice

In this order (details and acceptance in `tasks.md`, series `PCC`):

1. **PCC-1 (S):** put the referral ID in **SCH-26** of the SIU messages the PAS already sends, and as `basedOn` in the FHIR Appointment, when the appointment has a referral.
2. **PCC-2 (M):** write the referral lifecycle in the spec as one state table with the guide's edge rules (decline after acceptance, cancel losing to completion, no-show rebooks or ends, deferred resumes), mapped to the six states we have, and say which of the guide's states we do not have and why.
3. **PCC-3 (M):** add `information_requested`, `deferred` and `cancel_requested` only if a service wants them; start by asking a referral-handling service. Do not add states on the strength of a draft.
4. **PCC-4 (M):** an outbound **referral status notice** (booked, declined, redirected, closed, no-show), as an outbox topic a referrer can subscribe to (the machinery exists: `POST /api/admin/subscriptions`), carrying the referrer's referral ID and ours. Needs a named recipient first.
5. **PCC-5 (S):** identifier types and assigners on everything sent outward: say which identifier it is and who assigned it; never invent an assigner. Country-neutral (a configured assigning-authority value), not ODS-only.
6. **PCC-6 (S, a decision):** whether to model a plan-pointer flag. The guide's own text calls it a draft proposal. Wait for a consumer.
7. **PCC-7 (S, a decision):** an anti-corruption layer and raw-bundle store for any external referral standard (BaRS, eRS) — the pattern is right whatever the standard; it applies if and when a board asks for it.
8. **PCC-8 (S):** reconcile the "no planned encounters" rule with `pre_admit`, which creates a `Planned` encounter. Decide, and record it.

**Do not adopt:** the clinical document halves; copying safeguarding detail into any message; any state the owning service has not asked for; anything from the guide as if it were authoritative.

## Not checked

The pages not named above; IHE 360X itself (the guide's account of it was not verified against the supplement); the NHS England BaRS and eRS documents; any Welsh specification (Digital Health and Care Wales, the Welsh Clinical Portal, the National Data Resource); whether the HL7 v2 listener accepts any referral message (it routes ADT, DFT, MFN and SIU, from reading `src/hl7v2/listener.rs`; I did not look for a referral message type such as REF or OMG); and whether SCH-26 is accepted by the target systems. Our "not found" statements are text searches of `src/` and `migrations/`.

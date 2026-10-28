# ISO 14971 risk management: how the hazard log maps (DRAFT, not a risk management file)

**This is not a risk management file and nothing here is certified or conformant.** It shows how the existing hazard log ([`clinical-safety.md`](clinical-safety.md)) lines up with the process in ISO 14971:2019 (*Medical devices, application of risk management to medical devices*), so that a person with the standard in hand can see what is covered and what is missing. It was written from knowledge of the standard's structure, not from a licensed copy; check every clause against the standard itself.

## Does it apply at all?

ISO 14971 is for medical devices. The DTAC answer to C1.1.1 is that this system is **not** one, as an engineering judgement: it computes no diagnosis, dose, triage or risk score that drives care ([`docs/.../2.0/index.md`](../docs/united-kingdom/national-health-service/digital-technology-assessment-criteria/2.0/index.md)). **Nobody qualified has confirmed that.** The [MHRA](https://www.gov.uk/government/organisations/medicines-and-healthcare-products-regulatory-agency) decides what is a medical device. Two features could change the answer and are why this page exists: the no-show probability (H-10, H-13, [`noshow-model.md`](noshow-model.md)) and the triage categories ([`triage`](../spec/09-api-surface.md)). If either is judged to be a device function, ISO 14971 (and IEC 62304 for the software life cycle, and a quality system such as ISO 13485) applies to that function. If neither is, the NHS England standards DCB0129 and DCB0160 still apply to an organisation making or deploying health IT, and they use their own scoring scheme.

## Mapping

| ISO 14971 step | What this repository has | What is missing |
|---|---|---|
| Risk management plan (clause 4) | `clinical-safety.md` says what is in scope: an administrative system whose events feed clinical systems | A plan with named responsibilities and competence, the criteria for acceptable risk, and the method for judging overall residual risk. No Clinical Safety Officer is named |
| Intended use and foreseeable misuse (clause 5) | Scope line in `clinical-safety.md`; `plan.md` (administrative, not clinical); `noshow-model.md` ("support only") | A written intended-use statement. Foreseeable misuse is not listed (for example, staff using the no-show figure to deny an appointment, which NFR-11 forbids and tests) |
| Hazard identification, hazardous situations, harm (clause 5) | Hazards H-01 to H-14, with cause | The log lists hazards and causes but not the **sequence of events** to a hazardous situation or the **harm** to a person. A clinician adds these |
| Risk estimation (clause 5) | Likelihood and severity columns (`L`, `S`), blank on purpose | A scheme (qualitative or semi-quantitative) and a clinician to fill it. No numbers have been invented |
| Risk evaluation against criteria (clause 6) | None | Acceptability criteria from the plan |
| Risk control (clause 7), in the order inherent safety by design, protective measures, information for safety | "Existing control" column: state machines, transactions, approvals by a different named person, refusal while the score is off, outbox retries | The log does not say which kind of control each is. Most are design or protective measures; user information (training, warnings) is mostly absent |
| Verification of risk controls | Tests named in the log (NFR-11, `new2_validation_test.rs`, the schema guard) | Several controls say "test not written" (H-01, H-02, H-03, H-07) |
| New hazards introduced by a control | Not looked for | A review of each control for the hazards it creates (for example, a standby backfill that offers a ring-fenced slot) |
| Overall residual risk and benefit-risk (clause 8) | None | A judgement by someone accountable |
| Risk management review before release (clause 9) | None | A review and a report (the DCB0129 Clinical Safety Case Report covers most of this) |
| Production and post-production information (clause 10) | Audit log, outbox dead letters, the metrics endpoint, the chaos tests, `production-readiness.md` | A route for incidents and complaints to reach the hazard log, and a defined review of the log when the code or the weights change. Alerting on dead letters is not built (H-11) |

## What would make this real

1. Decide, with the MHRA's guidance and a qualified person, whether any function is a medical device. Record the decision and who made it.
2. Name a Clinical Safety Officer and agree the plan and acceptability criteria.
3. Have a clinician add harms and score `L` and `S`.
4. Classify each control, write the missing tests, and review each control for new hazards.
5. Put the log under change control and add the post-production route.

None of that can be done by this repository alone; steps 1 to 3 need people, and the first needs a regulator's view.

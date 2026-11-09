# 20. Suspected conditions: tracking suspicion during referrals

**Status: the rules below are a draft for sign-off (WPAS-1a).** They are written from public sources, cited. Before they are relied on for reporting, a cancer services manager must confirm the cancer rules against the current Welsh Government guidance, and a lead for each other condition must say whether a target applies. Until then the PAS records and shows the waits; it does not claim to report performance against a national standard.

## The requirement

A PAS MUST track *suspicion* during referrals. A referral is often sent because someone suspects a condition, not because one has been diagnosed. The wait that matters to the patient starts at that suspicion, and it is easily lost: the referral is received, triaged, booked, rebooked, and nobody can see how long it has been since a clinician first suspected cancer. Tracking the suspicion makes that wait visible, keeps the patient on a list until the suspicion is resolved, and stops a patient's own cancel from quietly ending the pathway.

The PAS **records** a suspicion; it does not decide it. A clinician suspects, diagnoses, rules out and treats. Staff record the dates a clinician gives them, under their own name and the clinician's (FR-207, FR-209).

## Conditions

Each suspicion names one condition from a coded list. Cancer is the worked example, because Wales publishes a pathway for it. The others are conditions a referral is often made for on suspicion, before a diagnosis. The column "why" says what the referral is for, not clinical guidance:

| Code | Condition | Why suspicion is tracked | Published target (source) |
|---|---|---|---|
| `cancer` | Cancer | The Suspected Cancer Pathway counts from the point of suspicion | 62 days from the point of suspicion to first definitive treatment, for at least 75% of patients ([Welsh Government, suspected cancer pathway](https://www.gov.wales/suspected-cancer-pathway-quality-report-html)) |
| `obstructive_sleep_apnoea` | Obstructive sleep apnoea | Referred on suspicion for a diagnostic sleep study | None found: set by the service |
| `diabetes` | Diabetes | Referred on suspicion for confirmation and a plan; some presentations are urgent, which is the clinician's call | None found: set by the service |
| `cardiovascular_disease` | Cardiovascular disease | Referred on suspicion (heart failure, angina, arrhythmia) for tests and a specialist opinion | None found: set by the service |
| `parkinsons_disease` | Parkinson's disease | Referred on suspicion to a specialist for diagnosis | None found: set by the service |
| `alzheimers_disease` | Alzheimer's disease | Referred on suspicion for a memory assessment and diagnosis | None found: set by the service |

The list is closed and coded (an enum with a database `CHECK` and a drift test, QUAL-28). Adding a condition is a migration and a row here, so no free text ever stands in for a diagnosis. **The PAS MUST NOT invent a target** (FR-211): where none is published, the wait is shown and no breach is claimed.

## The clock

- **Start: the point of suspicion.** The date a clinician first suspected the condition, whatever route the patient came in by. It may be before the referral was received (a GP suspected it, then referred). It is not the date the referral arrived, which is where the RTT clock starts.
- **Stop: once, for one reason** (FR-209). For cancer the NHS Wales Data Dictionary lists three stops, and "as any of these conditions stop the clock ... only one date should be recorded" ([Date of SCP Clock Stop](https://www.datadictionary.wales.nhs.uk/worddocuments/dateofscpclockstop.htm)):
  - `treatment_started`: the start of first definitive treatment;
  - `downgraded`: the suspicion was ruled out (for cancer, a downgrade);
  - `died`: the patient died.

  A fourth stop, `entered_in_error`, removes a suspicion recorded by mistake; it is kept for the audit and left out of every count.
- **No pauses** (FR-208). The days waited are the real days, from the point of suspicion to the stop date or to today. The Single Cancer Pathway rules that took effect on 1 December 2020 removed adjustments and suspensions from a patient's pathway. Unlike the RTT clock, nothing pauses a suspicion's clock: not a holiday, not a missed appointment, not a patient's choice.
- **Open question for sign-off (WPAS-1a):** the NHS Wales Data Dictionary still asks reports to show results "with and without suspensions applied" and says suspensions before the decision to treat should be captured. This PAS records none, so its report is the "without" view only. If the current guidance keeps suspensions for reporting, they would be a separate record, never a pause.

## Cancer details

- **Tumour site.** A cancer suspicion carries one site from the Welsh "Primary Cancer Site Description" list, valid from 1 January 2021 ([NHS Wales Data Dictionary](https://www.datadictionary.wales.nhs.uk/worddocuments/primarycancersitedescription.htm), last changed by DSCN 2023/45): `01` Head and Neck, `02` Upper GI, `03` Lower GI, `04` Lung, `05` Sarcoma, `06` Skin (exc BCC), `07` Brain/CNS, `08` Breast, `09` Gynaecological, `10` Urological, `11` Haematological (exc Acute Leukaemia), `12` Acute Leukaemia, `13` Children's, `14` Unknown Primary, `98` Other. Other conditions carry no site (FR-210).
- **Unique Pathway Identifier.** Optional, up to 24 characters, as the Welsh data sets key a pathway by it (with the NHS number, local patient identifier and provider organisation code).
- **A patient's own cancel is held for a person** (FR-214), as it already is on a service a clinician has listed (FR-58): a cancel reply on an appointment booked from a referral with an open cancer suspicion keeps the slot and creates a call task.

## What the PAS shows

- **The open list** (FR-212): every open suspicion, longest wait first, with its days waited, its target if it has one, and whether it is within 7 days of the target or past it. The 7 days is a display aid for whoever works the list, not a rule.
- **The report** (FR-213): for suspicions whose clock stopped in a period, per condition (and per tumour site for cancer), how many, how many within target, the share within target, and the median days. Each stopped suspicion is counted once; `entered_in_error` is left out; a condition without a target reports counts and the median only.

## Not built, and why

- **A decision about the patient.** The PAS never suspects, downgrades or ranks; it records what a named clinician decided.
- **National returns.** The Welsh data set's submission format (the "SCPC" return) is not produced. The data needed for it is held.
- **Targets other than cancer's,** until a lead for the condition names one and its source (WPAS-1a).
- **Suspensions** (see the open question above).

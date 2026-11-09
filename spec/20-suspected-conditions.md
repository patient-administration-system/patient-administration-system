# 20. Suspected conditions: tracking suspicion during referrals

**Status: the Welsh cancer rules are a draft for sign-off (WPAS-1a).** They are written from public sources, cited. Before they are relied on for reporting, a cancer services manager must confirm them against the current Welsh Government guidance, and a lead for each other condition must say whether a target applies. Until then the PAS records and shows the waits; it does not claim to report performance against a national standard.

## The requirement

A PAS MUST track *suspicion* during referrals. A referral is often sent because someone suspects a condition, not because one has been diagnosed. The wait that matters to the patient starts at that suspicion, and it is easily lost: the referral is received, triaged, booked, rebooked, and nobody can see how long it has been since a clinician first suspected cancer, sleep apnoea or dementia. Tracking the suspicion makes that wait visible, keeps the patient on a list until the suspicion is resolved, raises the referral's booking priority where the condition calls for it, and stops a patient's own cancel from quietly ending the pathway.

The PAS **records** a suspicion; it does not decide it. A clinician suspects, diagnoses, rules out and treats. Staff record the dates a clinician gives them, under their own name and the clinician's (FR-207, FR-209).

## What is fixed, and what is configured

**Fixed, in the core, for every condition** (FR-208, FR-209): the clock starts at the point of suspicion, never pauses, and stops once, for one reason, by a named person. No profile can change that.

**Configured, per condition, in a profile** (FR-220, FR-221, WPAS-8): which conditions are tracked, and for each one

| Setting | What it does | If absent |
|---|---|---|
| `code`, `name` | The code stored on each suspicion, and its name | Required |
| `subtypes` | A list (a `scheme`, its `source`, its codes and names) a suspicion may or, with `required`, must name: tumour sites for cancer, the type of diabetes or of dementia | No subtype is allowed |
| `target` | Days from the point of suspicion to the clock stopping, with the published `standard` and its `source` | No target: the wait is shown and never judged (FR-211) |
| `hold_patient_cancel` | A patient's own cancel by reply, on an appointment from a referral with an open suspicion of this condition, is held for a person to phone first (FR-214) | Not held |
| `booking_priority` | The waiting-list priority (`routine`, `urgent`, `two_week_wait`, `emergency`) a referral with an open suspicion is raised to; never lowered (FR-220) | Unchanged |

A profile is chosen with `PAS_SUSPICION_PROFILE`: a built-in name, or the path of an operator's own JSON file in the same shape. It is checked in full at start-up, and a mistake stops the server (FR-221). `GET /api/suspicion-profile` shows the active one.

**The rules a suspicion was recorded under are copied onto it** (its profile, target, cancel rule and booking priority), so a later change of profile never rewrites a wait that is already counting, and the report counts each suspicion against the target it was recorded with (FR-221). Names are shown only by the profile that defined them; a row recorded under another profile keeps its codes.

## The built-in profiles

| Profile | Conditions | Subtypes | Targets | Cancel held | Booking priority |
|---|---|---|---|---|---|
| `generic` (the default) | cancer, obstructive sleep apnoea, diabetes, cardiovascular disease, Parkinson's disease, dementia | diabetes: type 1, type 2, gestational, other; dementia: Alzheimer's, vascular, Lewy bodies, frontotemporal, mixed, not yet known (descriptive lists, not national code sets) | None | Cancer | None |
| `wales` | the same | as `generic`, and cancer: the Welsh primary cancer site list (required) | Cancer: 62 days | Cancer | Cancer: `urgent` |

The files are in `patient-administration-system-with-rust/profiles/suspected-conditions/`. The default names no country's code list and sets no target, because no country is the core (NFR-26). It still holds a suspected cancer's cancel for a person, because a missed cancer appointment is a safety matter wherever the PAS runs.

**Where the Welsh rules come from:**

- **Target:** at least 75% of patients start first definitive treatment within 62 days of the point of suspicion, from December 2020 ([Welsh Government, suspected cancer pathway](https://www.gov.wales/suspected-cancer-pathway-quality-report-html)).
- **Cancer sites:** the "Primary Cancer Site Description" list, valid from 1 January 2021 ([NHS Wales Data Dictionary](https://www.datadictionary.wales.nhs.uk/worddocuments/primarycancersitedescription.htm), last changed by DSCN 2023/45): `01` Head and Neck, `02` Upper GI, `03` Lower GI, `04` Lung, `05` Sarcoma, `06` Skin (exc BCC), `07` Brain/CNS, `08` Breast, `09` Gynaecological, `10` Urological, `11` Haematological (exc Acute Leukaemia), `12` Acute Leukaemia, `13` Children's, `14` Unknown Primary, `98` Other.
- **Unique Pathway Identifier:** optional on any suspicion, up to 24 characters, as the Welsh data sets key a pathway by it.

**Adding another country's pathway** is a profile, not code: for example England's Faster Diagnosis Standard would be a `cancer` target with its own source. Note that the core clock starts at the point of suspicion; a standard whose clock starts elsewhere (at the referral's receipt, say) is not the same measure, and the profile's `standard` must say so.

## The clock

- **Start: the point of suspicion.** The date a clinician first suspected the condition, whatever route the patient came in by. It may be before the referral was received (a GP suspected it, then referred). It is not the date the referral arrived, which is where the RTT clock starts.
- **Stop: once, for one reason** (FR-209). For cancer the NHS Wales Data Dictionary lists three stops, and "as any of these conditions stop the clock ... only one date should be recorded" ([Date of SCP Clock Stop](https://www.datadictionary.wales.nhs.uk/worddocuments/dateofscpclockstop.htm)); the PAS uses the same three for every condition:
  - `treatment_started`: the start of first definitive treatment;
  - `downgraded`: the suspicion was ruled out (for cancer, a downgrade);
  - `died`: the patient died.

  A fourth stop, `entered_in_error`, removes a suspicion recorded by mistake; it is kept for the audit and left out of every count.
- **No pauses** (FR-208). The days waited are the real days, from the point of suspicion to the stop date or to today. The Welsh Single Cancer Pathway rules that took effect on 1 December 2020 removed adjustments and suspensions from a patient's pathway. Unlike the RTT clock, nothing pauses a suspicion's clock: not a holiday, not a missed appointment, not a patient's choice.
- **Open question for sign-off (WPAS-1a):** the NHS Wales Data Dictionary still asks reports to show results "with and without suspensions applied" and says suspensions before the decision to treat should be captured. This PAS records none, so its report is the "without" view only. If the current guidance keeps suspensions for reporting, they would be a separate record, never a pause.

## What the PAS shows

- **The open list** (FR-212): every open suspicion, longest wait first, with its days waited, its target if it has one, and whether it is within 7 days of the target or past it. The 7 days is a display aid for whoever works the list, not a rule.
- **The report** (FR-213): for suspicions whose clock stopped in a period, per profile, condition, subtype and recorded target, how many, how many within target, the share within target, and the median days. Each stopped suspicion is counted once; `entered_in_error` is left out; a condition without a target reports counts and the median only.

## Not built, and why

- **A decision about the patient.** The PAS never suspects, downgrades or ranks; it records what a named clinician decided.
- **National returns.** The Welsh data set's submission format (the "SCPC" return) is not produced. The data needed for it is held.
- **Targets other than the Welsh cancer target,** until a lead for a condition names one and its source (WPAS-1a). An operator can add one in their own profile.
- **Suspensions** (see the open question above).
- **Changing a recorded suspicion's rules.** A suspicion keeps the rules it was recorded under; to apply a new profile's rules, a person stops it as `entered_in_error` and records it again.

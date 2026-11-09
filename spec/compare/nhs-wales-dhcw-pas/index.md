# Comparison: Welsh PAS (WPAS), Digital Health and Care Wales

Researched 2026-11-09 from the public page [WelshPAS e-learning resources](https://dhcw.nhs.wales/staff-resources/business-change/links/welshpas-elearning-resources/) (DHCW), public NHS Wales and Welsh Government material, and health-board freedom-of-information disclosures. The ideas it prompts are in [`plan.md`](../../../plan.md) §18 and `tasks.md` (series `WPAS`).

## What we could and could not read

The page lists nine e-learning items, all dated 15/04/2020. Every one of them (eight videos and a PDF on `elf.wales.nhs.uk`, plus the "What's New in 20.1" module) redirects to an NHS Wales Microsoft sign-in, so **we read only their titles and the page's summary of release 20.1, not their content**. We did not try to get past the sign-in. Everything below about how WPAS *behaves* is either from public sources (cited) or marked *inferred*. Treat it as a map of where to look, not as a specification of WPAS.

The page's items:

| Item | Kind | What the title tells us |
|---|---|---|
| Administration Tiles | video | A tiled staff home screen, probably by role *(inferred)* |
| Cancer Tracking Overview | video | A cancer-tracking module inside WPAS |
| Cancer Pathway Example | PDF | A worked pathway |
| Patient Enquiry | video | One screen that finds a patient and shows their PAS record *(inferred)* |
| Tracker 7 to Cancer Tracker Transition | video | Cancer tracking moved from a separate product ("Tracker 7") into WPAS |
| Tumour Site Browser | video | Pathways grouped and browsed by tumour site |
| Suspected Cancer Referral Record | video | A record for each suspected-cancer referral |
| Where to find Cancer Tracking | video | Navigation |
| What's New in 20.1 | e-learning | Release 20.1, below |

Release 20.1, as the page summarises it: an overview dashboard for admitted patients; bed management; a change to admission activity; ED discharge letters; clinical letters; and maternity items (social status, birth weight centile, Robson score, breastfeeding, fetal lie at onset of labour, record care plan, 36 to 38 week data collection, transfer during labour).

## What WPAS is (public sources)

WPAS is the national PAS that DHCW supplies to Welsh health boards. It records patient identity, outpatient appointments, letters and notes, waiting lists, medical records, inpatient stays and emergency attendances ([DHCW product directory](https://dhcw.nhs.wales/product-directory/dataand-information/welsh-patient-administration-system-wpas)). Health-board disclosures list a maternity module inside it, and say it connects to the other national systems: the Master Patient Index, the Welsh Radiology Information System and the Welsh Laboratory Information Management System. Clinicians reach it through the Welsh Clinical Portal. DHCW has merged separate instances into one (Betsi Cadwaladr), and its "WRAPPER" work is aimed at a single view of PAS data across Wales ([DHCW news](https://dhcw.nhs.wales/news/latest-news/category/dhcw-news/welsh-patient-administration-system-wpas/)).

## The Single Cancer Pathway (public rules)

This is the part of the page with the most behind it, and the biggest gap in this repository.

- **Clock start:** the *point of suspicion*, for every patient, whatever route they came in by. That is not the date a referral was received, which is where our RTT clock starts.
- **Clock stop:** the start of first definitive treatment ([NHS Wales Data Dictionary, "Date of SCP Clock Stop"](https://www.datadictionary.wales.nhs.uk/worddocuments/dateofscpclockstop.htm)).
- **Target:** at least 75% of patients start first definitive treatment within 62 days of the point of suspicion, from December 2020.
- **No adjustments:** the rules from 1 December 2020 removed adjustments and suspensions from a patient's pathway; the real wait is reported. Our RTT pathway pauses, which is the opposite. The data dictionary still asks reports to show results with and without suspensions, and an "SCP adjustments" data set existed briefly in January 2021 before being retired ([data set](https://www.datadictionary.wales.nhs.uk/worddocuments/singlecancerpathwayadjustmentsdataset.htm)). **The current rule needs checking** against the 2023 Welsh Government guidance on managing suspected-cancer patients, which we could not read in full.
- **Identifiers:** the data set keys a pathway by NHS number, local patient identifier, a 24-character Unique Pathway Identifier and the provider's organisation code.

## Where we overlap

| Area | WPAS | This repository |
|---|---|---|
| Outpatient appointments and letters | Core | Core, with a reminder ladder, many channels and 10 locales |
| Waiting lists and RTT | Core | Core: RTT clock with pause and resume, 18-week breach, validation campaigns |
| Inpatients and beds | Core; 20.1 adds an admitted-patient dashboard and bed management | ADT, ward and bed state, bed reservation, ward occupancy, discharge planning |
| Emergency attendances | Core; 20.1 adds ED discharge letters | Records an emergency encounter's triage category and measures the wait; no ED attendance record of its own, no discharge letter |
| Cancer tracking | A module: suspected-cancer referral record, tumour site browser, pathway tracking | **None.** A service can be flagged so a patient's own cancel goes to a person (FR-58), and that is all |
| Patient enquiry | One screen | Per-patient routes (appointments, waits, letters, history) with read auditing; no single "everything about this patient" view |
| Staff home screen | Administration tiles | None: the staff screens are separate pages |
| Patient identity | Linked to the national Master Patient Index | Delegated to `person-service` by design |
| Welsh language | The national PAS for Wales | Welsh (`cy-001`) is an AI draft, **not yet reviewed** by a first-language speaker |
| Maternity | A module, with the 20.1 data items | Out of scope (clinical record) |

## Assessment

**What WPAS has that we do not, and that fits our scope:**

1. **A cancer pathway tracker.** This is administrative: dates, a clock, a target and a list of who is close to breaching. The clinician decides the point of suspicion and the treatment; the PAS records the dates and tracks the wait. It is also the largest piece of work here, because its clock rules are different from RTT's and must not be bent to fit them.
2. **Patient enquiry.** Staff answering the phone need one place to see what the PAS knows about a patient. We have the parts; we have no page that puts them together.
3. **An admitted-patient overview.** We have ward occupancy and discharge planning; a ward-level page (who is in which bed, for how long, expected discharge) is the missing view.
4. **A staff home page by role.** Small, and it makes the rest findable.

**What WPAS has that we should not copy:** the maternity items (Robson score, birth weight centile, fetal lie, breastfeeding) are clinical observations. `spec/02-scope.md` excludes clinical records, and they would bring clinical-safety duties (DCB0129, DCB0160) with them. An ED discharge letter's *content* is clinical too; only sending it (the envelope, the recipient, the audit) would be ours.

**Where we lead:** open source and an open API (OpenAPI, FHIR R5, HL7 v2); patient-facing replies, links and offers; a safety case, DTAC and DPIA material; measurement of fairness; accessibility tests; CI that cannot skip database tests. WPAS is a national system with national integrations we do not have and should not pretend to.

**Limit of this comparison:** we read titles, not training content. Before building any WPAS task, a person who uses WPAS (a cancer tracker, a ward clerk, a booking clerk) should be asked what the screen actually does and what they wish it did.

## Advice

In this order (details and acceptance in `tasks.md`, series `WPAS`):

1. **WPAS-1, cancer pathway tracking**, first, because it is the largest gap and a missed cancer wait is a patient-safety matter. Start with the rules decision (WPAS-1a): read the 2023 guidance, confirm the current position on suspensions, and get a cancer services manager to sign the rules off. Only then build.
2. **WPAS-3, patient enquiry**, next: mostly assembling what exists, behind the read audit we already have.
3. **WPAS-4, the admitted-patient overview**, and **WPAS-5, the staff home page**, when a ward clerk or booking clerk asks.
4. **WPAS-6, sending an ED discharge letter**, only if an emergency department wants this PAS to send it.
5. **Not adopted:** the maternity data items (WPAS-7 records the decision).
6. **Before any of it:** get the Welsh translation reviewed (already tracked in `governance/translation-review.md`). A PAS offered in Wales without reviewed Welsh is not a credible alternative.

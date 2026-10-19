# Deprivation and age bands: the data route (settle with the health board early)

The equity splits in the analytics feed (CAP-11), the experiments (CAS-7) and the no-show fairness audit (CAP-12) all read two coarse bands per patient: an **age band** and a **deprivation quintile** (`person_demographics`). Without them every cell reads `unknown` and those reports say nothing about fairness. This is the plan for filling them, and what has to be decided by people.

## What is built

- `PUT /api/analytics/deprivation-lookup?dataset=...&loaded_by=...` loads a CSV of `postcode,quintile` supplied by the board (a header line is fine; bad lines are counted and skipped, never guessed at). The PAS ships **no data**. Each row keeps the dataset name and who loaded it. Loading is audited.
- `POST /api/analytics/demographics/refresh?limit=` (run it daily) takes patients who have appointments and no bands, reads the person record from person-service, derives the **age band from the date of birth** and the **quintile from the postcode**, and stores only the two bands with the source. The date of birth and postcode are used and not kept. A patient with no usable date or no matching postcode gets only the part that is known; the rest stays `unknown`, not guessed. Bands older than 90 days are refreshed.
- Convention in the PAS: **quintile 1 is the most deprived, 5 the least** (as WIMD ranks start at 1 = most deprived). The file the board loads must follow it.

## What people must settle

1. **Which index and which year.** For Wales, the Welsh Index of Multiple Deprivation (WIMD) is the natural choice; it is published by Welsh Government at small-area (LSOA) level, with quintiles in its outputs. Patients who live over the border or are not in the WIMD frame need a stated rule (`unknown`, or England's IMD, which is a different index and not comparable).
2. **The postcode to area file.** WIMD 2019 on StatsWales lists a "postcode to LSOA and local authority" lookup and a "postcode to WIMD rank" lookup ([StatsWales WIMD](https://statswales.gov.wales/Catalogue/Community-Safety-and-Social-Inclusion/Welsh-Index-of-Multiple-Deprivation)). The pages say WIMD 2019 data may be used under the Open Government Licence; **confirm the licence on the exact postcode file you download** (Royal Mail postcode data has its own terms, and the board's information governance team should sign this off). The Welsh Government stats inclusion team is the listed contact.
3. **Who builds the CSV.** A board analyst joins the postcode file to the quintile (quintile = ceiling of the LSOA's rank divided by 382 for 1,909 areas; check the method in the WIMD guidance) and loads it. The PAS does not do that join.
4. **Lawful basis and minimisation.** The PAS reads the postcode and date of birth that person-service already holds for care, to produce coarse bands for measuring equity. State that purpose in the privacy notice (DTAC-2) and the DPIA. Postcode district is not stored here.
5. **How often the index is refreshed.** WIMD is not annual; record the dataset name on each load so a report can say which one it used.
6. **Small areas and small numbers.** The feed already hides cells under 10 (secondary suppression included). A rural board with few patients per quintile will see many hidden cells; widen the period, not the privacy rule.

## Until it is settled

The reports run and say `unknown`. Do not read a fairness audit or an equity split as reassuring while most of its patients are `unknown`: the audit lists the share unknown, and a reviewer should treat a high share as a reason to wait.

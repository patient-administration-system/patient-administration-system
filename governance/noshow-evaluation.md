# Evaluating the no-show score: shadow first, then one clinic at a time

The real test of the score (CAP-12) is not whether it is accurate but whether it **raises attendance**, without harm to any group. The PAS is built to be evaluated that way, and to refuse to skip the steps.

## The design

1. **Shadow mode (the default for every clinic).** With the gate passed, the score is computed and stored for each upcoming appointment, but **staff see nothing and no call is created** for a clinic in shadow. The point is a record, made before each visit, of what the model expected.
2. **Check against what happened.** `GET /api/noshow/evaluation` shows *calibration*: the stored probability against the actual no-show rate, by tenth of predicted. It is the evidence for two decisions: whether the weights are any good, and whether the percentage may ever be shown (`PAS_NOSHOW_SHOW_PERCENT`). Read it next to the fairness audit (`/api/noshow/fairness-audit`).
3. **Stepped rollout by clinic.** A named person moves **one clinic** at a time to live: `PUT /api/noshow/rollout/{service_type}` with a note saying why this clinic now. The server refuses unless the gate is open and the clinic has at least `PAS_NOSHOW_SHADOW_MIN_DAYS` (28) of shadow data. Live means staff see the band and reasons, and the top few appointments a day (matched to call capacity) get a call task.
4. **The comparison is built in.** The evaluation shows, for each live clinic, the no-show rate **before and after** it went live, beside the rate at the clinics **still in shadow over the same period**. A fall in the live clinics that is not matched elsewhere is evidence the score helps; a fall in both is the season, not the score.
5. **Back out any time.** `stage: shadow` returns a clinic to shadow; a rejected review switches the whole thing off. Calls already on the list stay for a person to clear.

## No patient is denied support

The comparison is **between clinics over time**, never between patients in the same clinic. Every patient keeps the standard reminders, the reply options and the silent-patient ladder. The score only *adds* a phone call for some. Nobody is withheld from help in order to have a control group, and nothing in booking, rebooking, standby or offers reads the score (NFR-11, tested).

## What the evaluation cannot tell you

- A clinic-level before/after is weaker than a randomised trial; rollout order is chosen by people, and clinics differ. Pick the order for operational reasons that are not themselves related to no-show rates, and write the reason in the note.
- Cells under `k` are hidden (and a rate is shown only if every count behind it is at least `k`), so small clinics may show nothing for a while.
- It measures no-shows, not whether the *call* helped each person; the call outcome (reached, no answer, declined help) is on the task for that.
- A fall in no-shows is not by itself good: check the fairness audit and the equity splits after rollout (they need the demographic feed: `governance/deprivation-data-route.md`).

## Who decides

A fairness reviewer and a clinical safety reviewer, by name, before anything is live; a named person per clinic step. The code enforces the order, not the judgement.

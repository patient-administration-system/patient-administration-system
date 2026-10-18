# Translation review (OPEN)

All non-English catalogues in `patient-administration-system-ui-with-svelte/src/lib/messages/` are **machine-drafted and unreviewed by a first-language speaker**. Structure (keys, placeholders) is test-enforced; wording is not.

| Locale | Drafted by | Reviewed by | Date | Status |
|---|---|---|---|---|
| `cy-001` Welsh | AI, this session | | | **Must be reviewed before patients see it** |
| `de-001`, `de-de` German | AI, this session | | | Unreviewed |
| `ar-001` Arabic | AI draft, then edited by the maintainer's tooling | | | Unreviewed |
| `es-001` Spanish | AI draft, then edited | | | Unreviewed |
| `fr-001` French | AI draft, then edited | | | Unreviewed |
| `hi-001` Hindi | AI draft, then edited | | | Unreviewed |
| `zh-001`, `zh-cn` Simplified Chinese | AI draft, then edited | | | Unreviewed |

## Doubts raised while drafting (for the reviewer)

- **"Two-week wait"** (`priority.two_week_wait`) is an NHS England term for the urgent cancer referral pathway. No established term exists in Arabic, Hindi, Chinese, Spanish or French; every rendering is a literal attempt.
- **Calendar "previous/next {view}"** is built as "Previous: {view}" in Spanish, French, Arabic and Chinese to avoid gender and number agreement with a variable view name. A reviewer may prefer separate strings per view.
- **`calendar.agenda`** ("a list of upcoming items") is kept as "Agenda" in Welsh, German, Spanish and French; changed to a list-of-appointments sense in Arabic, Hindi and Chinese. Confirm.
- **Hindi** uses common transliterated terms (अपॉइंटमेंट, टेम्पलेट, चैनल, पोर्टल, वेरिएबल, कॉपी). A reviewer may prefer pure Hindi, for example "नियुक्ति".
- **Chinese** `waitlist.*` uses 等候名单 (waiting for treatment) rather than 候诊 (waiting to be seen). `patients.search` says 编号 rather than 标识符.
- **Welsh**: terminology follows NHS Wales usage as the drafter understood it (for example "apwyntiad", "Rhestr aros", "Porth diogel"). Mutations and register need a first-language check. `signin.error.idle` is colloquial.
- **German** uses "Sie" and inclusive wording ("Patientinnen und Patienten") in running text and "Patienten" in headings.
- **Product name** ("Patient Administration System") is translated literally in every language; it is not an established name.
- **Gender**: Spanish and French status words agree with "cita" (feminine) and "rendez-vous" (masculine) respectively.

## How to record a review
Fill in the table above, commit, and keep the reviewer's name out of the repository if they prefer (initials and a date are enough).

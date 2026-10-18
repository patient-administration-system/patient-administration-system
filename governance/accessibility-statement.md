# Accessibility statement (DRAFT, NOT PUBLISHED)

This is a draft of the statement DTAC D1.4.3 asks for. **It must not be published until the checks it describes have been done by people.** What is true today:

## Compliance status

We have not claimed conformance to WCAG 2.2 level AA. The UI is **not yet fully tested**.

## What has been checked, by machine

- axe-core, with the WCAG 2.0, 2.1 and 2.2 A and AA rule tags, finds no serious or critical issue on every page route, signed in as a patient and as an administrator, and on the sign-in page in English, Welsh, Arabic and Chinese (`patient-administration-system-ui-with-svelte/tests/e2e/a11y.spec.ts`).
- No page scrolls sideways at 390 pixels wide (`mobile.spec.ts`).
- The page declares its language and direction (`lang`, `dir`); Arabic is right to left.
- A skip link, labelled form fields and visible focus.

Automated checks find only part of the problems. They do not replace the manual work below.

## Not yet done

- Keyboard-only walk-through of every task.
- Screen-reader testing (NVDA, JAWS, VoiceOver, TalkBack).
- Zoom to 400% and text spacing checks.
- Review of the Lily calendar, board, chart and grid widgets by assistive-technology users.
- Testing with disabled patients and staff.

## Known issues

- Dates and times are shown as plain ISO values in UTC.
- Widget-internal text that Lily does not let us translate may remain in English in some languages.

## Contact and enforcement

To be completed by the deploying organisation: a way to report a problem, a response time, and the route to the enforcement body.

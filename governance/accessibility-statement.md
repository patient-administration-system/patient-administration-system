# Accessibility statement (DRAFT, NOT PUBLISHED)

This is a draft of the statement DTAC D1.4.3 asks for. **It must not be published until the checks it describes have been done by people.** What is true today:

## Compliance status

We have not claimed conformance to WCAG 2.2 level AA. The UI is **not yet fully tested**.

## What has been checked, by machine

- axe-core, with the WCAG 2.0, 2.1 and 2.2 A and AA rule tags, finds no serious or critical issue on every page route, signed in as a patient and as an administrator, and on the sign-in page in English, Welsh, Arabic and Chinese (`patient-administration-system-ui-with-svelte/tests/e2e/a11y.spec.ts`).
- No page scrolls sideways at 390 pixels wide (`mobile.spec.ts`), and the pages a patient reaches from a message (the reply link, the offer link, sign-in) do not at 320 pixels either, which is what 400% zoom of a 1280-pixel window gives (WCAG 1.4.10, `accessibility-walkthrough.spec.ts`).
- With the WCAG 1.4.12 text-spacing overrides applied (line height 1.5, letter spacing 0.12em, word spacing 0.16em, paragraph spacing 2em), nothing on those pages is clipped or scrolls sideways (same file).
- The reply link and the offer link can be used from the keyboard alone: the skip link is the first stop, focus is always visible, a button works with Enter or Space, the reasons for cancelling work with the arrow keys and Tab leaves them, and nothing traps focus (same file). The same pages also work with scripts turned off (`reply-link.spec.ts`).
- The page declares its language and direction (`lang`, `dir`); Arabic is right to left.
- A skip link, labelled form fields and visible focus.

Automated checks find only part of the problems. They do not replace the manual work below.

## Not yet done

- A keyboard-only walk-through of the staff tasks (the patient reply and offer links are covered above).
- Screen-reader testing (NVDA, JAWS, VoiceOver, TalkBack).
- Zoom to 400% and text spacing on the signed-in staff pages (the patient links are covered above).
- Review of the Lily calendar, board, chart and grid widgets by assistive-technology users.
- Testing with disabled patients and staff.

## Known issues

- Dates and times are shown as plain ISO values in UTC.
- Widget-internal text that Lily does not let us translate may remain in English in some languages.

## Contact and enforcement

To be completed by the deploying organisation: a way to report a problem, a response time, and the route to the enforcement body.

## A fault the keyboard checks found (2026-10-19)

The reply and offer pages sent `Referrer-Policy: no-referrer`. With that policy a browser sends `Origin: null` on a plain form post, which SvelteKit refuses as cross-site (403). A patient who tapped a button before the page had finished loading, or with scripts off, got an error instead of their answer. The policy is now `same-origin` (the link's token still never goes to another site), and a test submits the form with scripts off.

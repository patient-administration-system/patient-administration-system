# Locales

Applies to the SvelteKit UI. Requirements NFR-4 and NFR-5 ([`07-non-functional-requirements.md`](07-non-functional-requirements.md)).

## Codes

`<language>-<region>`, lowercase, hyphenated. `-001` is the UN M.49 code for "world": a language with no regional variant. **No bare two-letter code appears anywhere**: not as a locale, a catalogue file name, a URL prefix or a label.

| Code | Language | Direction | Catalogue status |
|---|---|---|---|
| `en-001` | English (source of truth, default) | ltr | Source |
| `cy-001` | Welsh | ltr | AI draft, **unreviewed** |
| `de-001`, `de-de` | German | ltr | AI draft, unreviewed (identical text) |
| `ar-001` | Arabic | **rtl** | AI draft, unreviewed |
| `es-001` | Spanish | ltr | AI draft, unreviewed |
| `fr-001` | French | ltr | AI draft, unreviewed |
| `hi-001` | Hindi | ltr | AI draft, unreviewed |
| `zh-001`, `zh-cn` | Simplified Chinese | ltr | AI draft, unreviewed (identical text) |

Review status is tracked in [`../governance/translation-review.md`](../governance/translation-review.md). Welsh must be reviewed by a first-language speaker before patients see it.

## URLs

Every page lives under its locale: `/cy-001/my/appointments`. The routes are not moved: the universal `reroute` hook strips a supported prefix before matching, so `/cy-001/signin` is served by `/signin`.

- A bare GET (`/signin`) is redirected (302) to the prefixed URL, choosing the locale from the `locale` cookie, then `Accept-Language` (nearest supported language: `de-DE` gives `de-de`, `es-MX` gives `es-001`), then `en-001`.
- A two-letter prefix (`/en/signin`) is **not** a locale: it is an ordinary path that gains `/en-001` and then finds no page.
- Never localized: `/api/*`, `/_app/*`, `/assets/*`, `/signout`, `/signin/oidc`, `/signin/oidc/callback`, static files. The OIDC provider needs one fixed callback address, so the language is carried in the login transaction.
- The root layout derives the locale from the URL in a universal load, so a client-side navigation to another language re-renders in it. (A server-only layout load would not re-run.)
- The document carries `lang` and `dir` from the server, before any script runs.

## Catalogues

One flat JSON file per locale in `src/lib/messages/`: dotted keys, `{name}` placeholders, formatted with Unicode MessageFormat 2 (`@sveltia/i18n`). Inserted values are wrapped in bidi isolates, so a Latin service name cannot scramble an Arabic sentence. Fallback is the locale, then English, then the key.

## Rules that are tests

| Rule | Test |
|---|---|
| Locale codes are well-formed; no bare two-letter name | `tests/unit/locale-names.test.ts` |
| Every locale has every key, no extras, the same placeholders, no empty string | `tests/unit/i18n.test.ts` |
| Nothing equals English unless allow-listed (names, and words that really are the same) | `tests/unit/i18n.test.ts` |
| No visible English outside `t()` in any Svelte template | `tests/unit/catalogue-coverage.test.ts` |
| URL, direction, language menu, browser language | `tests/unit/locales.test.ts`, `tests/e2e/locales.spec.ts` |

## Known gaps

- Text inside the Lily widgets that the widgets do not let us set (the data grid's and the board's own labels) may remain English in some languages.
- Dates and times are shown as ISO values in UTC.
- Content written by administrators (message templates) exists in one language each (CAP-9).
- Right-to-left layout has been checked for overflow only, not by an Arabic reader.
- No `sitemap.xml` with `hreflang` alternates yet.

# National and regional assumptions in the PAS (QUAL-40, NFR-26)

**Status: first pass, 2026-10-29, by search and reading, not a full audit.** The target is worldwide (NFR-26): no country, region or federation is the core, and a national scheme is an optional profile or adapter over a neutral core. This register lists what the first pass found that assumes one, what already is neutral, and what each needs. Nothing below has been changed by this register; "Done" appears only where the code already holds.

How it is kept honest: `tests/qual40_national_assumptions_test.rs` counts national names (`NHS`, `Wales`, `WIMD`, `SNOMED`, `HIPAA`, `Europe/London` and nine more) in each file under `src/` and fails if a file gains one. The counts are in [`national-assumptions-baseline.csv`](national-assumptions-baseline.csv) (41 files, 106 mentions on 2026-10-29). Lower a number when a file is made neutral; raise one only with a line here saying why. It is text only: it stops a national form being added, it does not show one is absent.

## Already neutral (kept)

| Area | Where | Why it holds |
|---|---|---|
| Phone numbers | `contact::normalise_e164`, `PAS_DEFAULT_COUNTRY_CODE` | stored as E.164; a national form is accepted only with an operator-set country code |
| Time zone | `PAS_TIMEZONE` (an IANA name), `Intl` in the UI | configurable; see the finding below on the fallback |
| Dates and numbers in the UI | `Intl.DateTimeFormat` and `Intl.NumberFormat` with the user's locale | no hard-coded order or separator; data is ISO 8601 |
| Currency | `Iso4217`, `Money` in billing | an ISO 4217 code on every amount |
| Locale codes | NFR-4: `<language>-<region>`, `-001` for the world | no bare two-letter code; Arabic (rtl), Hindi and Chinese ship |
| FHIR and HL7 | FHIR R5 base, HL7 v2 | international standards; Patient and Practitioner mapping assumes no name order, identifier system or address shape (`api/fhir/r5.rs`) |
| Addresses | `Address`: `line1`, `line2`, `city`, `state`, `postal_code`, `country`; one of city, postal code or country is enough | a country with no postal codes is valid |
| Deprivation data | operator loads a CSV (`governance/deprivation-data-route.md`); the PAS ships none | no index is built in (the convention that 1 is most deprived is fixed) |
| SMS | GSM 7-bit table and a fallback for other scripts (`communication/sms_length.rs`) | the standard table, not a national one |

## Findings

| # | Assumption | Where | What it needs | State |
|---|---|---|---|---|
| 1 | The only national identifier check is the **NHS number** (modulus 11), in the core request path and as its own route `POST /api/validate/nhs-number` (DTAC-8). | `validation/mod.rs`, `api/rest/handlers_contact.rs`; other schemes (NIR, TSI, IHI, HCN) were removed to person-service in v0.44.0 | One identifier-system registry: a scheme is a profile entry (system URI, check function) an operator enables; the NHS number is the first entry, not a route of its own. | Open |
| 2 | Identity proofing methods are a fixed list including `nhs_login` and `postal_code`. | `identity/mod.rs` `METHODS` | A configurable list; `nhs_login` becomes one national-login adapter among others (eID, Aadhaar, MyGovID and the like). | Open |
| 3 | The time zone **falls back to Europe/London** when `PAS_TIMEZONE` is unset or invalid, silently. | `inbound/messages.rs` `timezone()` | Refuse to start with no valid zone, or use UTC and say so; never a national default. | Open |
| 4 | Message dates are built from **hand-written English and Welsh day and month names**; every other language takes one of those. | `inbound/messages.rs` (`EN_DAY`, `CY_DAY` and the like) | Format with CLDR data for the patient's locale (the `icu` crates), as the UI already does with `Intl`. | Open: a patient reading Arabic, Hindi or Chinese gets English or Welsh names |
| 5 | Referral-to-treatment: `DEFAULT_BREACH_WEEKS = 18`, the module and API named RTT, England and Wales clock-pause rules. | `models/rtt.rs`, `waitlist/`, `db/repositories/rtt.rs` | A waiting-time target clock with a named rule set (RTT is one profile: target, which events pause or stop it). The default must be set, not 18. | Open; the per-pathway `breach_weeks` is already configurable |
| 6 | Default message wording must exist **in English and Welsh** (CAS-1, tested). Welsh is required by a Welsh language standard; no other locale has the same test. | `communication/reminders.rs` | Defaults in every shipped locale, with the Welsh requirement as one jurisdiction's rule (a configured required-locale set). | Open |
| 7 | The triage category is coded with **SNOMED CT UK** acuity concepts and the Welsh EDDS category; the only triage system is `mts`. | `triage/interchange.rs`, `triage/mod.rs` `SYSTEMS` | Other acuity schemes (CTAS, ESI, ATS, SATS) and their codes as registry entries; the five internal categories stay the core. | Open |
| 8 | The only national FHIR profile is **Wales R4** (`/fhir/r4/...`, `DataStandardsWales-*`); the IHE profiles are international. | `api/fhir/r4.rs`, `ihe_scheduling.rs` | A profile registry (UK Core, US Core, others) so a region adds a mapping without touching the core; Wales becomes one. | Open |
| 9 | The age and deprivation bands read a **postcode** lookup, and the audit text is built on WIMD. | `analytics/demographics.rs`, `analytics/dims.rs` | A small-area code (postcode, ZIP, PIN, municipality) and a named index; the band set is an operator choice. | Open |
| 10 | A name is **family and given**: the upstream person shape, the HL7 v2 encoder and the UI view helpers read `family` and `given`. | `upstream/person_client`, `hl7v2/encode.rs`, UI `lib/views.ts` | A `text` (full name) path first, family and given optional; the FHIR R5 mapping already keeps a mononym or text name. person-service owns the shape. | Partly: R5 mapping done; HL7 and UI open |
| 11 | Compliance documents and wording are **UK**: DTAC, DCB0129, UK GDPR, PECR, DSPT; the consent model says GDPR/HIPAA. | `governance/`, `communication/consent.rs`, `models/consent.rs` | A regime map (GDPR, UK GDPR, HIPAA, PIPEDA, LGPD, POPIA, PDPA and others) naming which controls each satisfies; marketing-consent rules a configured regime, not PECR in the code. | Open |
| 12 | The seed and the examples use UK and US names (`NHS England`, `Medicare`, `Aetna`) as illustrations. | comments and fixtures | Mixed examples from several regions. Cosmetic. | Open, low |
| 13 | Languages: eight, all AI drafts, none for several of the world's largest. | `spec/locales.md` | A process to add a locale, and a rule that a region's language is not second to a UK or US one. | Open |

## Not looked at (so not cleared)

The Svelte UI beyond the number, date and name helpers; the Rust frontend crate; letters and print (paper size, address block order, right-to-left layout in PDFs); the HL7 v2 messages other than the name; the report and analytics wording; the schema's column names and constraints beyond addresses; the Keycloak realm; documentation and the spec's own examples. Each needs the same read.

## Acceptance for QUAL-40

The register exists (this file) and the first lint exists. **Not met yet:** each finding above made a profile or configuration, or recorded as kept with a reason; a test per class (identifiers, names, addresses, dates and calendars, currency, code systems, waiting-time rules, regulatory text, locales) rather than the one name ratchet; the "not looked at" list read.

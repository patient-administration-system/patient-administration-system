# NHS Digital Technology Assessment Criteria (DTAC) v2.0: self-assessment

Self-assessment of this repository against the NHS Digital Technology Assessment
Criteria form v2.0, using the [checklist](checklist.md). Assessed 2026-10-18. It is an engineering assessment by the maintainer's tooling, not a
submission, an attestation, or legal advice, and nobody has audited it.

## How to read this

- **Scope.** The product is the Patient Administration System as shipped from this repository:
  - the Rust server, `patient-administration-system-with-rust` v0.44.0
  - the Loco server-rendered operator front-end, `patient-administration-system-frontend` v0.2.0
  - the SvelteKit patient and staff UI, `patient-administration-system-ui-with-svelte` v0.1.0
- **Open source, self-hosted.** The maintainer does not run a hosted service, employ staff with access to any deployment's data, or hold a DSPT, ICO, CQC or Cyber Essentials record for it. Several DTAC answers belong to *whoever deploys* the system. Where that is the case, this document says so instead of answering for them.
- **Evidence rule.** An answer is "Yes" or "Met" only where a file, test or command in this repository shows it. Everything else is "Not provided", "Unable to confirm" or "Not met". Paths are relative to the repository root.
- **Status words.** *Met*, *Partly met*, *Not met*, *Not applicable*, *Not provided* (the artefact does not exist) and *Unable to confirm*.

## Result

| Section | Kind | Result today |
|---|---|---|
| A. Company information | Non-assessed | Mostly not applicable: there is no company behind the repository |
| B. Value proposition | Non-assessed | Answered below |
| C1. Clinical safety | Assessed, pass/fail | **Not met.** No DCB0129 process, hazard log or Clinical Safety Officer |
| C2. Data protection | Assessed, pass/fail | **Not met.** No DPIA, privacy notice, ICO or DSPT evidence. The technical controls exist |
| C3. Technical security | Assessed, pass/fail | **Not met.** No Cyber Essentials, penetration test or documented MFA plan |
| C4. Interoperability | Assessed, pass/fail | **Partly met.** Open APIs and FHIR exist. NHS number is delegated and NHS login is not used |
| D1. Usability and accessibility | Scored | Weak. No user testing or accessibility statement, and WCAG 2.2 AA is untested |

**This product would not pass DTAC today.** Almost all of the shortfall is missing *organisational artefacts* (documents, certificates, a named clinician). Little of it is missing code. The [gap list](#gaps-in-priority-order) at the end separates the two kinds and links each gap to a task in [`tasks.md`](../../../../../tasks.md).

## A. Company information (non-assessed)

| Code | Answer |
|---|---|
| A1 | Not applicable. No registered company; maintained by an individual, see git history |
| A2 | Patient Administration System |
| A3 | Server 0.44.0, operator front-end 0.2.0, UI 0.1.0 (see each `Cargo.toml` / `package.json`) |
| A4 | Software as a Service *if you host it*; as shipped, self-hosted server software (Other) |
| A5–A8 | Not provided |
| A9 | Not applicable |
| A10 | Not applicable |
| A11, A12 | Not applicable. The maintainer is not a CQC-registered provider |

## B. Value proposition (non-assessed)

| Code | Answer |
|---|---|
| B1 | Workforce Support or Management, and Patient Care or Support |
| B2 | Hospital administration: admit, transfer and discharge (ADT), appointments and scheduling, waiting lists and referral-to-treatment (RTT) clocks, bed and ward occupancy, letters and patient notifications, billing and coverage. Patients can sign in to see their own appointments. See [`README.md`](../../../../../README.md) |
| B3 | Intended users: administrative and clinical staff, and patients. **No benefit has been validated** and no evaluation or trial has been run |
| B4 | Partly provided. Data-flow descriptions exist in [`AGENTS/share/overview.md`](../../../../../patient-administration-system-with-rust/AGENTS/share/overview.md) and [`AGENTS/architecture.md`](../../../../../patient-administration-system-with-rust/AGENTS/architecture.md). There is no DPIA-grade data-flow diagram and no user journey map |

## C1. Clinical safety (assessed)

| Code | Answer | Status |
|---|---|---|
| C1.1.1 | **No**, as an engineering judgement. The system administers records and schedules. It computes no diagnosis, dose, triage or risk score that drives care. *Not formally determined by a clinician or regulator* (the regulator for medical devices is the [MHRA](https://www.gov.uk/government/organisations/medicines-and-healthcare-products-regulatory-agency)). A future no-show risk score (see `plan.md`) would need this re-asked | Unable to confirm |
| C1.1.2 | No | Not applicable |
| C1.2 | **Yes, probably.** Admission, transfer, discharge, bed status and appointment events feed clinical systems in near real time (HL7 v2 and an event stream), and a wrong bed or appointment is a patient-safety hazard | Unable to confirm |
| C1.2.2 | **No** | Not met |
| C1.2.3 | Clinical risk management system | Not provided |
| C1.2.4 | Clinical Safety Case Report and Hazard Log | Not provided |
| C1.2.5 | Clinical Safety Officer | Not provided. No CSO is named |

**What exists that a hazard log would draw on:** a state machine that rejects invalid transitions ([`src/models/appointment.rs`](../../../../../patient-administration-system-with-rust/src/models/appointment.rs)), transactional audit and outbox writes, and a test suite of over 400 tests. None of this is a DCB0129 process.

## C2. Data protection (assessed)

| Code | Answer | Status |
|---|---|---|
| C2.1 | DSPT: **Unable to Confirm.** The maintainer has no DSPT return. A deploying NHS organisation or supplier needs its own | Unable to confirm |
| C2.2 | **Yes.** The product processes personal data. Demographics are held by the upstream `person-service`; this system holds patient identifiers (`person_id`), encounters, appointments, consents, letters, billing and audit rows | n/a |
| C2.2.1 | ICO registration | Not provided |
| C2.2.2 | DPIA | Not provided |
| C2.2.3 | Privacy notice / transparency information | Not provided |
| C2.2.4 | Terms and licence. The Rust crates declare SPDX licences in `Cargo.toml`; **no `LICENSE` file is in the repository** | Not provided |
| C2.2.5 | **Depends on the deployment.** The software makes no call to a non-UK service at run time except what the deployer configures (the optional OTLP exporter and webhook targets). Build time downloads Swagger UI from GitHub, and images come from public registries | Unable to confirm |
| C2.2.6 | Not applicable until a deployment is chosen | Not applicable |

**Technical controls that support a DPIA** (all in [`AGENTS/share/privacy.md`](../../../../../patient-administration-system-with-rust/AGENTS/share/privacy.md)):
masked patient views, a JSON subject-access export (`GET /api/patients/{id}/export`), a consent model with revocation (`Consent`, `ConsentType`, `ConsentStatus`), and an audit log written in the same transaction as every change. **Known limit:** `require_consent` is not yet wired into every handler.

## C3. Technical security (assessed)

| Code | Answer | Status |
|---|---|---|
| C3.1 | Cyber Essentials certificate | Not provided |
| C3.2 | Cyber Security Charter for Suppliers to the NHS: **No** | Not met |
| C3.3 | External penetration test, OWASP Top 10, last 12 months | Not provided |
| C3.4 | Software Security Code of Practice: **Unable to Confirm.** Partial evidence is listed below | Unable to confirm |
| C3.5 | MFA plan: the UI signs people in through Keycloak OIDC (authorization code with PKCE), which supports MFA, but the shipped realm ([`podman/keycloak/pas-realm.json`](../../../../../podman/keycloak/pas-realm.json)) does **not** require it and no plan is written down | Not met |
| C3.5.1 | Supplier accounts with privileged access | Not applicable. The maintainer has no access to any deployment |
| C3.6 | Logging and reporting requirements: **Yes, with a caveat.** Every state change writes an `audit_log` row (entity, action, old and new JSON, user id, IP, user agent, time). The user id comes from an `X-User-Id` request header "recorded as-is" ([`AGENTS/share/auditability.md`](../../../../../patient-administration-system-with-rust/AGENTS/share/auditability.md)), so it is only as trustworthy as the calling service | Partly met |

**Partial evidence for C3.4**

- Dependencies are locked (`Cargo.lock`, `pnpm-lock.yaml`) and were updated on 2026-10-18.
- `cargo audit` currently reports **6 vulnerabilities**, all transitive and none fixable by this repository: `zip` (3) only at build time through `utoipa-swagger-ui`; `quick-xml` (2) through Loco's `opendal`; `rsa` (1, no upstream fix) through `sqlx-mysql`.
- Per-IP rate limiting (`src/api/rest/rate_limit.rs`) and a shared bearer-token check on the API (`src/api/rest/auth.rs`).
- The SvelteKit UI sets a strict Content-Security-Policy and security headers, keeps tokens on the server (a backend-for-frontend with an httpOnly session cookie), and refuses cross-role access (browser tests in `patient-administration-system-ui-with-svelte/tests/e2e/sign-in.spec.ts`).
- **Missing:** no CI pipeline, no `cargo-deny` policy, no `#![forbid(unsafe_code)]`, no secret scanning, no SBOM, no vulnerability disclosure policy (`SECURITY.md`).
- **Weak point to state plainly:** per-user authentication now exists (PASETO tokens via `authentication-verifier`, DTAC-4) but is **opt-in**: with `PAS_PASETO_KEYS_URL` unset the API still falls back to one shared bearer token or none (`trusted-caller mode` is logged at start-up). Set `PAS_REQUIRE_AUTH=true` in any real deployment. The repository ships a service-token issuer (`pas-token-issuer`) and the UI has been run against it end to end in the browser suite; it does not issue per-person tokens, and the private key's production storage is the operator's to arrange. The UI's server enforces who may see what and is the only supported client for patients.

## C4. Interoperability (assessed)

| Code | Answer | Status |
|---|---|---|
| C4.1 | **Yes.** REST/JSON API, FHIR R5 resources, HL7 v2 ADT output, bulk JSON/XML/TSV/CSV import and export | n/a |
| C4.1.1 | REST with OpenAPI 3 (`utoipa`, served at `/swagger-ui` and `/api-docs/openapi.json`); HL7 FHIR R5 for Patient, Encounter, Appointment, Practitioner and others (`src/api/fhir/`); HL7 v2 for ADT messages (`src/streaming/hl7v2_publisher.rs`). These are the standards NHS and hospital systems already exchange | Met |
| C4.1.2 | GDS API best practice: **Cannot confirm.** The API is documented and openly available (source and OpenAPI), but it has not been reviewed against the GDS guidance (versioning, error shape, pagination) | Unable to confirm |
| C4.1.3 | Basis for third parties: the source is public, the OpenAPI document is served by every instance, and the FHIR surface follows the published resource definitions | Met |
| C4.2 | **Yes.** Patient identity matters for every exchange | n/a |
| C4.2.1 | NHS number: **Partly.** Identity is delegated to `person-service`, which supports the NHS number among other national identifiers ([`AGENTS/national-person-identifiers.md`](../../../../../patient-administration-system-with-rust/AGENTS/national-person-identifiers.md)). This repository stores a `person_id` UUID and does not itself require or validate an NHS number | Partly met |
| C4.2.2 | Personal Demographics Service integration: **No.** Nothing in this repository calls PDS | Not met |
| C4.2.3 | How patients are identified: by the upstream `person_id`; duplicate and merge events arrive by signed webhook (`/api/upstream/person-merged`, HMAC-checked) and are applied to PAS-owned rows. Data quality is the upstream service's responsibility | Partly met |
| C4.2.4 | NHS login for patient sign-in: **No.** Patients sign in with Keycloak OIDC | Not met |
| C4.2.5 | DAPB3051 identity verification: **Not evidenced** | Unable to confirm |
| C4.2.6 | How users are authenticated: OIDC authorization-code flow with PKCE against Keycloak; the ID token is verified (RS256 signature, issuer, audience, expiry, nonce); sessions are server-side with an httpOnly `__Host-` cookie, 8-hour maximum and 30-minute idle timeout; a patient sees only the record named by the verified `person_id` claim. **Known gap:** nothing proves the person signing in *is* that patient. An administrator sets the `person_id` attribute by hand after identity checks done outside this system | Partly met |

## D1. Usability and accessibility (scored)

| Code | Answer | Status |
|---|---|---|
| D1.1 | Existing systems and pathways: described in [`README.md`](../../../../../README.md) and the Rust project's `spec.md`. No user journey or instructions for use | Partly provided |
| D1.2 | Testing with intended users: **No** | Not met |
| D1.3 | Accessible Information Standard: **Cannot confirm.** The UI has a skip link, labelled form fields and uses the NHS England themes from the Lily design system, but the standard has not been reviewed | Unable to confirm |
| D1.4 | Web application: **Yes** | n/a |
| D1.4.1 | WCAG 2.2 AA: **No.** Not tested. No automated (axe) or manual audit exists for this UI | Not met |
| D1.4.2 | Plan and timescale: none written. See `tasks.md` (DTAC-12) | Not provided |
| D1.4.3 | Accessibility statement | Not provided |
| D1.5 | Service availability over the past 12 months: **no deployment has run, so there is no figure.** Health endpoints (`/api/health`, `/_health`) exist for a deployer to monitor | Not applicable |

## Supporting documents

| Code | Document | Status |
|---|---|---|
| A12 | CQC report | Not applicable |
| B4 | User journeys and data flows | Partly provided |
| C1.1.1 | Pre-acquisition questionnaire | Not applicable (assessed as not a medical device) |
| C1.2.3, C1.2.4 | Clinical risk management system, Clinical Safety Case Report | Not provided |
| C1.2.4 | Hazard Log | Not provided |
| C2.2.1 | ICO registration | Not provided |
| C2.2.2 | DPIA | Not provided |
| C2.2.3 | Privacy notice | Not provided |
| C2.2.4 | Terms and conditions or licence | Not provided |
| C3.1 | Cyber Essentials certificate | Not provided |
| C3.3 | Penetration test summary | Not provided |
| D1.1 | User journeys | Partly provided |

## Gaps in priority order

"Document" means the gap cannot be closed by code alone. Task numbers refer to [`tasks.md`](../../../../../tasks.md).

| # | Gap | Kind | Task |
|---|---|---|---|
| 1 | Name a Clinical Safety Officer and run DCB0129: hazard log, safety case report | Document, needs a clinician | DTAC-1 |
| 2 | DPIA, privacy notice and data-flow diagram | Document | DTAC-2 |
| 3 | Add a `LICENSE` file and a `SECURITY.md` disclosure policy | Document, small | DTAC-3 |
| 4 | Per-user authentication on the API, and a trustworthy audit user id: **code done, deployment open** | Code + deployment | DTAC-4 |
| 5 | Require MFA in the Keycloak realm and write the MFA plan | Config and document | DTAC-5 |
| 6 | CI with fmt, clippy `-D warnings`, tests, `cargo-deny`, SBOM, secret scan | Code | DTAC-6 |
| 7 | WCAG 2.2 AA: axe over every route, keyboard and screen-reader pass, accessibility statement | Code and document | DTAC-12 |
| 8 | Wire `require_consent` into every handler that needs it | Code | DTAC-7 |
| 9 | NHS number validation and PDS lookup, or a written alternative | Code | DTAC-8 |
| 10 | Patient identity proofing before linking a sign-in to a `person_id`; assess NHS login | Code and document | DTAC-9 |
| 11 | External penetration test; Cyber Essentials; sign the Cyber Security Charter | Organisational | DTAC-10 |
| 12 | Usability test with real patients and staff | Organisational | DTAC-11 |

Items 1, 2, 11 and 12 need people and money, not code. No amount of engineering moves C1, C2 or C3 to a pass without them.

## Keeping this current

Re-assess at every release. Update the date at the top, change a status only when a file, test or command in the repository supports it, and never write "Met" for something that is only intended.

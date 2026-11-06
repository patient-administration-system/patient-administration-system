# 3. Stakeholders and users

| Role | Keycloak role | What they do |
|---|---|---|
| Patient | `patient` | Signs in; reads their own appointments and portal messages |
| Staff | `staff` | Calendar, waitlist, schedule, patient search |
| Administrator | `admin` (also has staff access) | As staff, plus editing message templates |
| Operating Department Practitioner (ODP) | `staff` | Works across the three areas of the operating theatre: anaesthetics, surgery and recovery. Works alongside anaesthetists, manages airways, prepares patients for anaesthesia, assists the surgical team, maintains patient safety, anticipates problems, responds to emergencies and looks after patients as they wake up after surgery. This system records none of that clinical work (§2); an ODP uses it with the same access as other staff |
| Integrating system | API bearer token | Calls the REST API; receives events |
| Clinical Safety Officer | n/a (not yet named) | Owns the hazard log (see DTAC-1) |
| Deploying organisation | n/a | Hosts, holds DSPT/ICO/Cyber Essentials, owns its DPIA |

A patient is linked to a record by the `person_id` attribute on their identity-provider account. That link is set by an administrator after identity checks made outside this system (see §16, question 2).

## Role against area (OFH-6)

Checked 2026-11-02 against `patient-administration-system-ui-with-svelte/src/routes/**/+page.server.ts`, `src/lib/server/guard.ts` and `patient-administration-system-with-rust/src/api/rest/user_auth.rs`. The realm has three roles (`patient`, `staff`, `admin`); there are no finer capability flags.

| Area | Patient | Staff | Administrator | Where it is enforced |
|---|---|---|---|---|
| My appointments, my messages, reply links (`/my/*`, `/r/*`, `/o/*`) | own records | no | no | UI: the patient's own `person_id` scope |
| Appointments, schedule, waiting list, waits, history, patient search | no | yes | yes | UI: `requireStaff`; API: the UI's service token |
| Tolerances: read | no | yes | yes | UI: `requireStaff` |
| Tolerances: set, reset, acknowledge an alert | no | no | yes | UI: `requireAdmin`; **API: `administrator_only`** (a verified token needs `access=admin` or `svc=true`) |
| Reminder and message templates (`/admin/reminders`) | no | no | yes | UI: `requireAdmin` |
| Operational metrics, notification subscribers (`/api/admin/*`) | no | no | yes | **API: `administrator_only`** |

**What this shows, plainly.**

- Only the tolerance, metrics and subscriber routes are refused for a non-administrator at the API. For every other area the staff-against-administrator split is made by the UI alone, because the UI's backend-for-frontend holds a trusted service token (`svc=true`), which the API's policy lets do anything. A caller who reaches the API with a verified token that has `access=write` is limited by the token's write level, not by a staff or administrator role.
- There is no per-area capability (scheduling, waiting list, billing, analytics, clinical data). Whether a board needs them is a question for the first deployment (OFH-6); none has been asked for.
- The first administrator is created in the identity provider (Keycloak), not in this system, so the "once only, secret-protected bootstrap" idea has nothing to apply to here. The development realm ships an `admin` user with a development password; `governance/mfa-plan.md` and `SECURITY.md` cover production.
- Tests: `tests/unit/guard.test.ts` and `tests/unit/route-guards.test.ts` (UI); `tests/user_auth_test.rs` (API, database). A test per row of this table was not written: the rows above are from reading the code.


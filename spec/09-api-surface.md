# 9. API surface

The generated contract is the OpenAPI document served at `/api-docs/openapi.json` with Swagger UI at `/swagger-ui/`, committed as [`openapi.json`](openapi.json). It lists every route and method (268 operations: 244 REST, 24 FHIR), with path parameters and the three response classes. It describes the JSON request body of the 105 REST routes that take a typed one (as a schema with its field types and doc comments) and the credential each route needs: a bearer token (PASETO v4.public) everywhere except `/api/health` (open), `/api/upstream/*` (an HMAC in `x-upstream-signature`) and `/api/write-back/*` (a bearer token or the carrier's signed credential). and the query parameters of the 2 REST routes that take a typed query (name, type and doc comment of each field). **It does not yet describe the typed `data` of a response, the FHIR and proxy routes' bodies (free-form `Value`), `text/plain` for HL7, or examples** (the rest of QUAL-34 in `tasks.md`). A test fails when a typed JSON body or typed query is added without being described. A test fails when a route is added without being documented and when `openapi.json` is stale (regenerate with `UPDATE_OPENAPI=1 cargo test -p patient-administration-system openapi`). This chapter lists the endpoints this repository's recent work depends on.

| Method and path | Purpose |
|---|---|
| `GET /api/health` | Liveness and database check |
| `GET /api/appointments?from&to` | Appointments starting in `[from, to)`, at most 366 days |
| `GET /api/patients/{id}/appointments` | One patient's appointments, earliest first |
| `GET /api/patients/{id}/letters?channel=` | One patient's letters, newest first; `channel=portal` is the portal inbox |
| `GET /api/letter-templates`, `GET /api/letter-templates/{id}` | Read templates |
| `PUT /api/letter-templates/{id}` | Edit a template (syntax checked, audited) |
| `POST /api/letters/generate` | Render and send one letter |
| `GET /api/waitlist?service=` | Waitlist for a service |

Authentication: per-user PASETO v4.public tokens verified offline (`PAS_PASETO_KEYS_URL`); or, if that is unset, one shared bearer token; or none in trusted-caller mode. With per-user tokens the audit user id is the verified `sub`, a client `X-User-Id` is discarded, and only a `svc=true` service may add `X-On-Behalf-Of`. Verified by `tests/user_auth_test.rs`. `PAS_REQUIRE_AUTH=true` refuses to start without per-user auth. The UI's server still enforces what a patient may see (NFR-3).

Routes use `{param}` syntax (axum 0.8).

**Versioning.** The API has one path, `/api`. A client asks for a version with the `Accept-Version` request header (for example `Accept-Version: 2`); a request without the header gets the current version. Paths MUST NOT carry a version: there is no `/api/v1`, `/api/v2` and so on. A version the server does not support is refused with 406 and a body naming the versions it does. The server states the version it used in an `API-Version` response header. (NFR-17)

## Triage category interchange (NEW-24)

`GET /fhir/Observation?encounter=Encounter/<id>` returns each triage assessment of an emergency encounter as a FHIR R5 `Observation` (searchset, oldest first). Round trip: `triage::interchange::category_of_observation` reads the category back (unit test for all five).

| PAS category | SNOMED CT concept (UK preferred term) | Manchester level | Wales EDDS Triage Category |
|---|---|---|---|
| `immediate` | 1064891000000107 (Immediate resuscitation level emergency care) | Red, Priority 1 | 01 |
| `very_urgent` | 1064911000000105 (Very urgent level emergency care) | Orange, Priority 2 | 02 |
| `urgent` | 1064901000000108 (Urgent level emergency care) | Yellow, Priority 3 | 03 |
| `standard` | 1077241000000103 (Standard level emergency care) | Green, Priority 4 | 04 |
| `non_urgent` | 1077251000000100 (Non-urgent level emergency care) | Blue, Priority 5 | 05 |

The `Observation` has `status` `final`, `category` `survey`, `code.text` "Urgent and Emergency Care Acuity", `subject` the patient, `encounter`, `effectiveDateTime` the assessment time, `performer.display` the assessor, `method.text` the system and version, and `valueCodeableConcept` with two codings: the SNOMED CT concept and the PAS's own code (`urn:patient-administration-system:triage-category`).

**Sources:** NHS Wales Data Dictionary, [Urgent and Emergency Care Acuity](https://www.datadictionary.wales.nhs.uk/worddocuments/urgentandemergencycareacuity.htm) (the concept ids and the Manchester mapping; valid from 1 August 2024), [Triage Category](https://www.datadictionary.wales.nhs.uk/worddocuments/triagecategory.htm) (codes 01 to 05; 06 "See and Treat" is not a Manchester category and is not mapped) and [Triage Score](https://www.datadictionary.wales.nhs.uk/worddocuments/triagescore.htm). Read 2026-10-23.

**Not verified, so not claimed:**
- The same concept ids in the England ECDS acuity set (no England document that lists them was found).
- A SNOMED CT concept for the Observation's own `code` (it carries text only; the community candidate 713011005 stays unverified).
- **HL7 v2:** no field for a triage category was verified. PV2 visit priority (table 0217) is Emergency, Urgent or Elective, and PV2 acuity (table 0432) describes clinical state at admission (acute, chronic, critical and so on); neither is a triage category, so the PAS does not map one into them. Revisit if a receiving system names a field.

## Audit export and read auditing (NEW-9)

- A successful `GET` of `/api/patients/{id}` or anything under it, or of `/fhir/Patient/{id}`, writes one `read` row to `audit_log` for that patient (user, address and client from the request headers; the route with `{id}` in place of the id, never the query string). A failed read writes nothing; if the row cannot be written the read is refused. Not covered: searches and batch reads that do not name one patient in the path (`/api/patients/search`, `/fhir/Flag?patient=`, `/api/reasonable-adjustments/for-appointments`, reports and exports).
- `GET /api/audit/balp?from&to&entity_type&entity_id&limit` returns the audit rows of a window (at most 31 days; at most 500 rows) as a searchset Bundle of FHIR **R4** `AuditEvent`s in the shape of [IHE Basic Audit Log Patterns](https://profiles.ihe.net/ITI/BALP/) (BALP 1.1.4 targets R4; the PAS FHIR endpoint is R5, so this is not in its capability statement). The response is `application/fhir+json; fhirVersion=4.0`, and the export is itself audited (`export_balp`).
- Mapping: a `read` row is action `R`; otherwise a row with only a new value is `C`, with both `U`, with only an old value `D`; a row with neither is `E` and carries no BALP profile. `entity_type` `patient` adds the patient entity and the `Patient*` profile. The client agent (DICOM 110153) is the caller's address and client; the server agent (DICOM 110152) is this system and its `HOSTNAME`; the user agent (`IRCP`) is the audit user id.
- `meta.profile` names a BALP profile only when `audit_export::violations` finds nothing wrong, so a row with no client address, no server `HOSTNAME`, or an action outside create, read, update and delete is exported without the claim. **The check is written from the published profile pages; the FHIR validator has not been run against them.** The two published profile pages disagreed on which DICOM code is the client; the DICOM meaning (110153 Source Role ID, 110152 Destination Role ID) was used. Check against the validator before relying on the claim.

## IHE ITI Scheduling (NEW-10)

The [IHE ITI Scheduling profile](https://profiles.ihe.net/ITI/Scheduling/) (1.0.0) is FHIR R4 and the PAS FHIR endpoint is R5, so its operations are under `/fhir/r4` with their own CapabilityStatement (`GET /fhir/r4/metadata`) and are not in the R5 statement. Read from ITI-115, ITI-116, ITI-117 and the three OperationDefinitions on 2026-10-23.

| Operation | Request | Behaviour |
|---|---|---|
| `$find` (ITI-115) | `POST /fhir/r4/Appointment/$find`, a `Parameters` body; `?_count=` (default 10, at most 50) | Free one-to-one slots in the period (default 14 days, at most 90) for `patient-reference`, earliest first and in no other order (NFR-11), each recorded as a proposal good for an hour. A slot held for someone else, or overlapping the patient's own appointment, is left out. Returns a `searchset` Bundle of `proposed` Appointments. |
| `$hold` (ITI-116) | `POST /fhir/r4/Appointment/$hold` with `appointment-reference` | Keeps the slot for `PAS_HOLD_MINUTES` (default 10). The Appointment comes back `pending`. Anyone else's booking of that slot, through this API or `POST /api/slots/{id}/book`, is refused while the hold runs. No longer available answers 404 with an `OperationOutcome` of severity `fatal` and code `not-found`, as the profile asks. |
| `$book` (ITI-117) | `POST /fhir/r4/Appointment/$book` with `appointment-reference` (and `comment`) | Books the slot with the code behind `POST /api/slots/{id}/book` (owner clash, patient overlap, group-session and interpreter rules; audit and outbox event). The Appointment comes back `booked` with the real appointment id. A hold that has run out does not stop the booking while the slot is still free. |

A proposal is the Appointment's id until it is booked. **Not built, and refused by name rather than ignored:** modify and cancel through `$book` (`appointment-resource`), a patient who does not exist yet (`patient-resource`), group appointments, the specialty, visit-type, organization, reason, referral, timing and insurance filters, ITI-118 (find existing appointments), and paging. Group sessions are not offered by `$find`. The IHE pages publish no example messages for these operations, so the tests use messages written from the OperationDefinitions, not the profile's own examples.

## Event subscriptions (NEW-4)

An administrator registers who is told about changes: `POST /api/admin/subscriptions` with `topic` (`appointment` or `waiting-list`), `endpoint` (`https`, or `http` on localhost; no user name or password) and optionally `secret_env`, the name of an environment variable like `PAS_SUB_SECRET_PARTNER` that holds the signing secret (the secret is never stored). `GET /api/admin/subscriptions` lists them with their counts of pending, delivered and failed notifications; `DELETE /api/admin/subscriptions/{id}` switches one off and drops what it has not yet been sent. All three are administrator-only and audited. `GET /fhir/SubscriptionTopic` and `GET /fhir/Subscription` show the same in FHIR R5 (`channelType` `rest-hook`, `content` `id-only`).

- **Topics:** `appointment` is every outbox event whose type starts `Appointment`; `waiting-list` is every one that starts `Waitlist`. There is no referral topic because referrals publish no outbox event yet (QUAL-35).
- **Delivery:** each new outbox event is copied into one delivery per active subscriber of its topic (events from before the subscription began are not sent), so a change reaches each subscriber once. A failure is retried after 30 seconds, then 1, 2, 4 minutes and so on up to an hour, until `PAS_OUTBOX_MAX_RETRIES` attempts (default 10; 0 for no limit); then the delivery is `failed` and stays visible. A pass sends at most 16 per subscriber, so a subscriber with a backlog or a dead endpoint does not keep the others waiting. All state is in the database, so a restart loses nothing. Receivers should treat `X-PAS-Event-Id` as the idempotency key: a notification is repeated if the receiver's answer is lost.
- **Message:** an R5 `Bundle` of type `subscription-notification`: a `SubscriptionStatus` (`event-notification`, event number, topic) and, for an appointment event, the reference `Appointment/<id>` (`id-only`: no patient data in the message). `X-PAS-Event-Type` carries the event type.
- **Not built:** the handshake and heartbeat notifications, `query-event`, the R4 Backport (NEW-1), `full-resource` content, filters, a referral topic, and a block on endpoints that resolve to private addresses (the administrator who registers one is trusted). An event the global publisher dead-lettered before the subscription pass ran is not delivered.

## Clinic templates, slot types and capacity (NEW-8)

`POST /api/session-templates` (`schedule_id`, `name`, `created_by`) starts a template; `POST /api/session-templates/{id}/versions` proposes the next version (`weekday` 1 to 7, `start_time` in London local time, `slot_minutes` 5 to 240, `effective_from`, `lines` of `{slot_type, slots, ring_fenced}`, `proposed_by`); `POST /api/session-templates/{id}/versions/{version}/approve` (`approved_by`) approves it, and the approver must be a different person from the proposer. `POST /api/session-templates/{id}/generate` (`from`, `to`, `generated_by`; at most 92 days; not before today) makes the slots from the approved version in force on each day (the latest `effective_from` on or before it); repeating it adds none. `GET /api/session-templates?schedule_id=` lists templates with every version. `PUT /api/slots/{id}/ring-fence` holds a free future slot out of automatic offers (first available slot for a referral, rebooking options, IHE `$find`) or lifts the hold; a person can still book it by choosing it. All are audited and name the person.

`GET /api/capacity/report?service_type=&from=&to=&lookback_days=` compares slots by type in a forward window (default 28 days) with required slots per week: referrals received in the look-back (default 84 days) divided by its weeks, divided by one minus the observed non-attendance rate. **It is plain arithmetic, stated in the response, and is not NHS England's core model**: that model's formulas are in a spreadsheet and are not published as text, so sustainable waiting list size, percentile bands over demand and the split of missed appointments into reused and lost slots are not computed. The user guidance only defines the terms: required capacity is what stops the list growing, available capacity is what can be deployed, and the sustainable list size is the size that keeps patients seen in good time (<https://www.england.nhs.uk/wp-content/uploads/2018/11/demand-capacity-core-model-user-guidance.pdf>).

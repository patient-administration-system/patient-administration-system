# 9. API surface

The generated contract is the OpenAPI document served at `/api-docs/openapi.json` with Swagger UI at `/swagger-ui/`, committed as [`openapi.json`](openapi.json). It lists every route and method (268 operations: 244 REST, 24 FHIR), with path parameters and the three response classes. **It does not yet describe request bodies, query parameters, the typed `data` of a response, or which routes need a token** (QUAL-34 in `tasks.md`). A test fails when a route is added without being documented and when `openapi.json` is stale (regenerate with `UPDATE_OPENAPI=1 cargo test -p patient-administration-system openapi`). This chapter lists the endpoints this repository's recent work depends on.

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


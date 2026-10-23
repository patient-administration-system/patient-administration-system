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


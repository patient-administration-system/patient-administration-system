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

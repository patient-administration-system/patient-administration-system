# 9. API surface

The complete, generated contract is the OpenAPI document served at `/api-docs/openapi.json` with Swagger UI at `/swagger-ui/`. This chapter lists the endpoints this repository's recent work depends on.

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

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

Authentication: a shared bearer token, or none in trusted-caller mode. The API does not authenticate end users; the UI's server does (see NFR-3 and the DTAC gap DTAC-4).

Routes use `{param}` syntax (axum 0.8).

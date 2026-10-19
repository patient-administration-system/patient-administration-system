# Runbook

| Symptom | Likely cause | Check / action |
|---|---|---|
| Sign-in redirects to "not available" | Keycloak down, or `OIDC_ISSUER` wrong | `curl $OIDC_ISSUER/.well-known/openid-configuration` |
| Sign-in loops or fails with "did not complete" | Redirect URI not registered; clock skew; wrong client secret | Match `OIDC_REDIRECT_URI` (default `<origin>/signin/oidc/callback`) to the client; check the UI log for `oidc sign-in refused: <reason>` |
| Redirect URI shows `https://` on a plain-HTTP deployment | Missing `PROTOCOL_HEADER` | Set `PROTOCOL_HEADER=x-forwarded-proto` (the container does) |
| Everyone signed out after a UI restart | In-memory sessions | Use Redis for sessions, or accept it |
| Patient sees "not linked to a patient record" (403) | Account has no valid `person_id` | An administrator sets the `person_id` attribute on the Keycloak account |
| `/api/health` says `database: unreachable` | PostgreSQL down or `DATABASE_URL` wrong | Check the database |
| Events not reaching consumers | Publisher failing | `GET /api/admin/outbox/unpublished`, `GET /api/admin/outbox/dead-letters`; replay with `POST /api/admin/outbox/dead-letters/{id}/replay` |
| No reminders sent | Reminders off, no sender enabled, patient has no contact point, outside the lead window | `PAS_REMINDERS_ENABLED=true`; a channel provider set; check `pas::reminders` log lines |
| Same reminder sent twice | Should not happen (a letter is the marker) | Report it; include the appointment id and the `generated_letters` rows |
| Worker merge webhook returns 500 | Missing column rename | Apply all migrations (`pas-migrate up`) |
| Startup says "trusted-caller mode" | No `PAS_PASETO_KEYS_URL` and no `API_TOKEN` | Configure per-user auth and set `PAS_REQUIRE_AUTH=true` |
| Rotating the issuer key | Planned | See "Issuing API tokens" in deployment.md: new `kid`, publish both keys, switch, drop the old |
| Every request is 401 after enabling per-user auth | Keys not fetched (log: "could not fetch PASETO keys"), wrong issuer/audience, or an expired token | Check the keys URL is reachable; keys refresh on a timer |
| `cargo test` shows tests "ignored" | DB tests need PostgreSQL | `DATABASE_URL=… cargo test -- --ignored` |

## Who to tell

A patient-safety incident follows the deploying organisation's incident process and the Clinical Safety Officer. A suspected vulnerability follows [`../SECURITY.md`](../SECURITY.md).

## Proofing a patient's portal account (DTAC-9)

A patient who signs in to the portal is only an account until staff have checked who they are. Until a staff member records it, every `/my/*` page answers 403, whatever `person_id` the identity provider carries.

1. Check the patient's identity by your organisation's procedure (in person, a video ID check, a postal code).
2. Record it: `PUT /api/identity-proofings/{subject}` with `{"person_id": "<patient>", "method": "in_person", "proofed_by": "<your name>"}`, where `{subject}` is the account's id in the identity provider (Keycloak's user id).
3. To cut an account off (a mistaken link, a safeguarding concern): `DELETE /api/identity-proofings/{subject}?revoked_by=<your name>`. The portal refuses it at once; the history is kept.
4. To see who proofed an account and how: `GET /api/identity-proofings/{subject}`. Each proofing and revocation is also in the patient's audit trail.

## Switching on the no-show probability (CAP-12)

It is off, and the server will not turn it on for you. Read `governance/noshow-model.md` first.

1. Run the audit on real data: `GET /api/noshow/fairness-audit?days=365`. Read it by deprivation quintile and language (the bands come from `PUT /api/analytics/demographics/{person_id}`; until an upstream job fills them every cell reads `unknown`).
2. A named person records their fairness review: `POST /api/noshow/reviews` with `{"kind":"fairness","reviewer":"<name>","outcome":"approved","notes":"<what was looked at and why it is acceptable, 40+ characters>"}`.
3. The clinical safety officer records theirs the same way with `"kind":"clinical_safety"`.
4. Set `PAS_NOSHOW_SCORE=true` and restart. `GET /api/noshow/status` should say `active: true`.
5. To stop: `POST /api/noshow/reviews` with `"outcome":"rejected"` switches it off at once, without a restart.

Thresholds (`PAS_NOSHOW_MEDIUM`, `PAS_NOSHOW_HIGH`) and weights (`PAS_NOSHOW_WEIGHTS`, JSON) are configuration. A change to the weights is a new decision: re-run the audit and review again.

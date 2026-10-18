import { defineEnvVars } from "@sveltejs/kit/env";

const optional = { schema: (input: string | undefined) => input ?? "" };

export const variables = defineEnvVars({
  // The PAS REST API and its shared bearer token (server-side only).
  PAS_API_URL: optional,
  PAS_API_TOKEN: optional,
  // A file holding a short-lived service token, re-read as it rotates. Takes precedence over PAS_API_TOKEN.
  PAS_API_TOKEN_FILE: optional,
  // OpenID Connect against Keycloak (see ../podman/keycloak/pas-realm.json).
  OIDC_ISSUER: optional,
  OIDC_CLIENT_ID: optional,
  OIDC_CLIENT_SECRET: optional,
  OIDC_SCOPES: optional,
  OIDC_TENANT_ID: optional,
  OIDC_IDP_HINT: optional,
  OIDC_REDIRECT_URI: optional,
  OIDC_SESSION_HOURS: optional,
  OIDC_IDLE_MINUTES: optional,
  OIDC_SESSION_STORE: optional,
  REDIS_URL: optional,
  OIDC_END_SESSION: optional,
});

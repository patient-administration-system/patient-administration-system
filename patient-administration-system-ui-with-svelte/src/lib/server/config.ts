// Server-side BFF configuration (never bundled into the browser).

import * as env from "$app/env/private";
import { readFileSync } from "node:fs";
import type { OidcConfig } from "./oidc";

/** PAS REST API base URL and the shared bearer token the BFF adds to every call. */
export const PAS_API_URL = (env.PAS_API_URL || "http://localhost:8080").replace(/\/$/, "");

/**
 * The credential the BFF presents to the PAS API. With `PAS_API_TOKEN_FILE` it is read from that file
 * (re-read at most every 5 seconds), because a per-user-auth deployment uses a short-lived service
 * token that a sidecar rotates; otherwise `PAS_API_TOKEN`. An unreadable file yields no credential,
 * so the API refuses the call rather than the BFF sending a stale one.
 */
let cached: { at: number; value: string } | null = null;
export function serviceToken(): string {
  const file = (env.PAS_API_TOKEN_FILE ?? "").trim();
  if (!file) return env.PAS_API_TOKEN || "";
  const now = Date.now();
  if (cached && now - cached.at < 5_000) return cached.value;
  let value = "";
  try {
    value = readFileSync(file, "utf8").trim();
  } catch {
    value = "";
  }
  cached = { at: now, value };
  return value;
}

/** OpenID Connect settings, or `null` when `OIDC_ISSUER` and `OIDC_CLIENT_ID` are not both set. */
export function oidcConfig(): OidcConfig | null {
  const issuer = (env.OIDC_ISSUER ?? "").trim();
  const clientId = (env.OIDC_CLIENT_ID ?? "").trim();
  if (!issuer || !clientId) return null;
  return {
    issuer,
    clientId,
    clientSecret: (env.OIDC_CLIENT_SECRET ?? "").trim() || undefined,
    scopes: (env.OIDC_SCOPES ?? "").trim() || "openid profile email",
    tenantId: (env.OIDC_TENANT_ID ?? "").trim() || undefined,
    idpHint: (env.OIDC_IDP_HINT ?? "").trim() || undefined,
  };
}

/** Deployment options for the OIDC flow that are not part of the protocol. */
export function oidcOptions(): {
  redirectUri?: string;
  sessionSeconds: number;
  idleSeconds: number;
  endProviderSession: boolean;
} {
  const idleMinutes = Number.parseFloat(env.OIDC_IDLE_MINUTES ?? "");
  const hours = Number.parseFloat(env.OIDC_SESSION_HOURS ?? "");
  return {
    redirectUri: (env.OIDC_REDIRECT_URI ?? "").trim() || undefined,
    sessionSeconds: Math.round((Number.isFinite(hours) && hours > 0 ? hours : 8) * 3600),
    idleSeconds: Math.round((Number.isFinite(idleMinutes) && idleMinutes >= 0 ? idleMinutes : 30) * 60),
    endProviderSession: (env.OIDC_END_SESSION ?? "").trim() === "true",
  };
}

// OpenID Connect client for the BFF (HD-12): the authorization-code flow with PKCE
// against any standards-following provider — Microsoft Entra ID directly, or a
// Keycloak realm (which can itself broker Entra). Pure functions over `node:crypto`
// and an injected `fetch`, no SvelteKit imports, so they are unit-testable.
//
// What this does and does not trust:
// - the provider's metadata must name the issuer we configured (an Entra metadata
//   document with a `{tenantid}` template is refused: configure a tenant);
// - the ID token is verified here: RS256 only, signature against the provider's
//   published keys, then `iss`, `aud`, `exp`, `nbf`, `nonce` and (when configured)
//   `tid`. `alg: none`, HMAC, and any other algorithm are refused;
// - the access token is NOT interpreted by the BFF: it is passed to the API as a
//   bearer, and the API verifies it (the Keycloak verifier). Nothing here grants
//   access by itself.
//
// Tokens never reach the browser; see `oidc-session.ts`.

import { createHash, createPublicKey, createVerify, randomBytes, timingSafeEqual } from "node:crypto";

type FetchFn = typeof fetch;

/** Why a step failed. The code is safe to log; messages never carry tokens. */
export class OidcError extends Error {
  constructor(
    public readonly code: "metadata" | "issuer_mismatch" | "token_endpoint" | "id_token" | "jwks" | "state" | "denied",
    message: string,
  ) {
    super(message);
    this.name = "OidcError";
  }
}

/** Operator configuration (see `config.ts` for the environment variables). */
export interface OidcConfig {
  /** The issuer URL, e.g. `https://login.microsoftonline.com/<tenant>/v2.0`. */
  issuer: string;
  clientId: string;
  /** Present for a confidential client; a public client (PKCE only) has none. */
  clientSecret?: string;
  /** Space-separated scopes; `openid` is always added. */
  scopes: string;
  /** Require this `tid` claim (Microsoft Entra tenant id). */
  tenantId?: string;
  /** A hint that skips the provider's chooser, e.g. Keycloak `kc_idp_hint=entra`. */
  idpHint?: string;
}

/** The parts of the discovery document we use. */
export interface ProviderMetadata {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  jwks_uri: string;
  end_session_endpoint?: string;
}

/** A JSON Web Key (RSA public). */
export interface Jwk {
  kty: string;
  kid?: string;
  use?: string;
  n?: string;
  e?: string;
  [k: string]: unknown;
}

/** The ID-token claims we read. */
export interface IdTokenClaims {
  iss: string;
  sub: string;
  aud: string | string[];
  exp: number;
  iat?: number;
  nbf?: number;
  nonce?: string;
  name?: string;
  preferred_username?: string;
  email?: string;
  tid?: string;
  oid?: string;
  roles?: string[];
  /** Keycloak realm roles. */
  realm_access?: { roles?: string[] };
  /** PAS person id, mapped from the Keycloak user attribute `person_id`. */
  person_id?: string;
}

/** What the token endpoint returns. */
export interface TokenResponse {
  access_token: string;
  id_token?: string;
  refresh_token?: string;
  expires_in?: number;
  token_type?: string;
}

export function b64url(buf: Buffer): string {
  return buf.toString("base64url");
}

/** A URL-safe random token (state, nonce, session ids). */
export function randomToken(bytes = 32): string {
  return b64url(randomBytes(bytes));
}

/** A PKCE verifier and its S256 challenge (RFC 7636). */
export function pkcePair(): { verifier: string; challenge: string } {
  const verifier = randomToken(48);
  const challenge = b64url(createHash("sha256").update(verifier).digest());
  return { verifier, challenge };
}

/** Where to send the browser to authenticate. */
export function authorizationUrl(
  meta: ProviderMetadata,
  cfg: OidcConfig,
  p: { state: string; nonce: string; challenge: string; redirectUri: string },
): string {
  const scopes = new Set(["openid", ...cfg.scopes.split(/\s+/).filter(Boolean)]);
  const q = new URLSearchParams({
    response_type: "code",
    client_id: cfg.clientId,
    redirect_uri: p.redirectUri,
    scope: [...scopes].join(" "),
    state: p.state,
    nonce: p.nonce,
    code_challenge: p.challenge,
    code_challenge_method: "S256",
  });
  if (cfg.idpHint) q.set("kc_idp_hint", cfg.idpHint);
  const sep = meta.authorization_endpoint.includes("?") ? "&" : "?";
  return `${meta.authorization_endpoint}${sep}${q.toString()}`;
}

/** Fetch and check the provider's discovery document. */
export async function discover(fetchFn: FetchFn, issuer: string): Promise<ProviderMetadata> {
  const url = `${issuer.replace(/\/$/, "")}/.well-known/openid-configuration`;
  let res: Response;
  try {
    res = await fetchFn(url, { headers: { accept: "application/json" } });
  } catch {
    throw new OidcError("metadata", "the provider's metadata could not be fetched");
  }
  if (!res.ok) throw new OidcError("metadata", `metadata answered ${res.status}`);
  const meta = (await res.json()) as Partial<ProviderMetadata>;
  if (typeof meta.issuer !== "string" || meta.issuer.includes("{")) {
    // Entra's "common" and "organizations" metadata carry a `{tenantid}` template.
    throw new OidcError(
      "issuer_mismatch",
      "the provider's issuer is missing or a template; configure a specific tenant",
    );
  }
  if (meta.issuer !== issuer.replace(/\/$/, "") && meta.issuer !== issuer) {
    throw new OidcError("issuer_mismatch", "the provider names a different issuer");
  }
  for (const f of ["authorization_endpoint", "token_endpoint", "jwks_uri"] as const) {
    if (typeof meta[f] !== "string") {
      throw new OidcError("metadata", `metadata has no ${f}`);
    }
  }
  return meta as ProviderMetadata;
}

/** The provider's published signing keys. */
export async function fetchJwks(fetchFn: FetchFn, meta: ProviderMetadata): Promise<Jwk[]> {
  let res: Response;
  try {
    res = await fetchFn(meta.jwks_uri, { headers: { accept: "application/json" } });
  } catch {
    throw new OidcError("jwks", "the signing keys could not be fetched");
  }
  if (!res.ok) throw new OidcError("jwks", `keys answered ${res.status}`);
  const body = (await res.json()) as { keys?: Jwk[] };
  if (!Array.isArray(body.keys)) throw new OidcError("jwks", "no keys in the key set");
  return body.keys;
}

function decodeSegment<T>(segment: string): T {
  return JSON.parse(Buffer.from(segment, "base64url").toString("utf8")) as T;
}

function sameString(a: string, b: string): boolean {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

/** What the ID token must satisfy. */
export interface IdTokenExpectation {
  issuer: string;
  audience: string;
  nonce: string;
  tenantId?: string;
  /** Seconds since the epoch; injectable for tests. */
  now?: number;
  /** Clock skew tolerated, in seconds. */
  skewSeconds?: number;
}

/**
 * Verify an ID token and return its claims, or throw {@link OidcError}.
 * RS256 only; the key is chosen by `kid`.
 */
export function verifyIdToken(idToken: string, keys: Jwk[], expect: IdTokenExpectation): IdTokenClaims {
  const parts = idToken.split(".");
  if (parts.length !== 3) throw new OidcError("id_token", "not a compact JWS");
  const [encodedHeader, encodedClaims, encodedSignature] = parts as [string, string, string];
  let header: { alg?: string; kid?: string };
  let claims: IdTokenClaims;
  try {
    header = decodeSegment(encodedHeader);
    claims = decodeSegment(encodedClaims);
  } catch {
    throw new OidcError("id_token", "malformed token");
  }
  if (header.alg !== "RS256") {
    throw new OidcError("id_token", "only RS256 ID tokens are accepted");
  }
  const candidates = keys.filter((k) => k.kty === "RSA" && (k.use === undefined || k.use === "sig"));
  const key = header.kid
    ? candidates.find((k) => k.kid === header.kid)
    : candidates.length === 1
      ? candidates[0]
      : undefined;
  if (!key) throw new OidcError("id_token", "no matching signing key");
  let ok: boolean;
  try {
    const publicKey = createPublicKey({ key: key as never, format: "jwk" });
    ok = createVerify("RSA-SHA256")
      .update(`${encodedHeader}.${encodedClaims}`)
      .verify(publicKey, Buffer.from(encodedSignature, "base64url"));
  } catch {
    ok = false;
  }
  if (!ok) throw new OidcError("id_token", "bad signature");

  const now = expect.now ?? Math.floor(Date.now() / 1000);
  const skew = expect.skewSeconds ?? 60;
  if (typeof claims.iss !== "string" || claims.iss !== expect.issuer) {
    throw new OidcError("id_token", "wrong issuer");
  }
  const aud = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  if (!aud.includes(expect.audience)) throw new OidcError("id_token", "wrong audience");
  if (typeof claims.exp !== "number" || claims.exp + skew < now) {
    throw new OidcError("id_token", "expired");
  }
  if (typeof claims.nbf === "number" && claims.nbf - skew > now) {
    throw new OidcError("id_token", "not yet valid");
  }
  if (typeof claims.nonce !== "string" || !sameString(claims.nonce, expect.nonce)) {
    throw new OidcError("id_token", "wrong nonce");
  }
  if (typeof claims.sub !== "string" || claims.sub === "") {
    throw new OidcError("id_token", "no subject");
  }
  if (expect.tenantId && claims.tid !== expect.tenantId) {
    throw new OidcError("id_token", "wrong tenant");
  }
  return claims;
}

async function tokenRequest(
  fetchFn: FetchFn,
  meta: ProviderMetadata,
  cfg: OidcConfig,
  form: Record<string, string>,
): Promise<TokenResponse> {
  const body = new URLSearchParams({ ...form, client_id: cfg.clientId });
  if (cfg.clientSecret) body.set("client_secret", cfg.clientSecret);
  let res: Response;
  try {
    res = await fetchFn(meta.token_endpoint, {
      method: "POST",
      headers: {
        "content-type": "application/x-www-form-urlencoded",
        accept: "application/json",
      },
      body,
    });
  } catch {
    throw new OidcError("token_endpoint", "the token endpoint could not be reached");
  }
  if (!res.ok) throw new OidcError("token_endpoint", `token endpoint answered ${res.status}`);
  const tokens = (await res.json()) as Partial<TokenResponse>;
  if (typeof tokens.access_token !== "string") {
    throw new OidcError("token_endpoint", "no access token in the response");
  }
  return tokens as TokenResponse;
}

/** Exchange an authorization code (with the PKCE verifier) for tokens. */
export function exchangeCode(
  fetchFn: FetchFn,
  meta: ProviderMetadata,
  cfg: OidcConfig,
  p: { code: string; verifier: string; redirectUri: string },
): Promise<TokenResponse> {
  return tokenRequest(fetchFn, meta, cfg, {
    grant_type: "authorization_code",
    code: p.code,
    redirect_uri: p.redirectUri,
    code_verifier: p.verifier,
  });
}

/** Use a refresh token to get a fresh access token. */
export function refreshTokens(
  fetchFn: FetchFn,
  meta: ProviderMetadata,
  cfg: OidcConfig,
  refreshToken: string,
): Promise<TokenResponse> {
  return tokenRequest(fetchFn, meta, cfg, {
    grant_type: "refresh_token",
    refresh_token: refreshToken,
    scope: cfg.scopes,
  });
}

/**
 * A safe place to return to after sign-in: a same-site absolute path only.
 * Anything else (another origin, a protocol-relative `//host`, a backslash trick,
 * a control character) becomes `/`.
 */
export function safeReturnPath(raw: string | null | undefined): string {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//") || raw.startsWith("/\\")) {
    return "/";
  }
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f\\]/.test(raw)) return "/";
  return raw;
}

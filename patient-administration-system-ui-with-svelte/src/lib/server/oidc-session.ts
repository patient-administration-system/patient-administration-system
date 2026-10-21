// Server-side storage for OIDC logins (HD-12): the in-flight login transactions
// (state, nonce, PKCE verifier) and the signed-in sessions (tokens). The browser
// holds only an opaque, random, httpOnly cookie id; the tokens stay here.
//
// One process, in memory. That is deliberate for now: a restart signs everyone
// out (they sign in again with one click), and with several front-end replicas a
// shared store (or sticky sessions) is required — stated in the operations guide.
// Pure and clock-injectable, so it is unit-tested without a server.

import {
  OidcError,
  fetchJwks,
  discover,
  randomToken,
  refreshTokens,
  type IdTokenClaims,
  type Jwk,
  type OidcConfig,
  type ProviderMetadata,
} from "./oidc";

import type { SessionStore } from "./session-store";

type FetchFn = typeof fetch;

/** The httpOnly cookie that names an OIDC session (host-locked via `__Host-`). */
export const OIDC_COOKIE = "__Host-pas_oidc";
/** The short-lived cookie that binds a callback to the browser that started it. */
export const OIDC_TX_COOKIE = "__Host-pas_oidc_tx";

/** Cookie attributes: httpOnly, Secure, host-locked, sent on top-level navigations. */
export const OIDC_COOKIE_OPTIONS = {
  path: "/",
  httpOnly: true,
  secure: true,
  sameSite: "lax",
} as const;

/** How long a login may take between redirect and callback. */
export const TX_TTL_MS = 10 * 60 * 1000;

/** A login in flight. Single use. */
export interface LoginTransaction {
  nonce: string;
  verifier: string;
  returnTo: string;
  redirectUri: string;
  /** The reader's UI locale, for the redirect back with an error. */
  locale: string;
  expiresAt: number;
}

/** A signed-in session. Server-side only. */
export interface OidcSession {
  id: string;
  accessToken: string;
  refreshToken?: string;
  idToken?: string;
  /** When the access token expires, in ms since the epoch. */
  accessExpiresAt: number;
  /** The session's hard end, in ms since the epoch. */
  expiresAt: number;
  /** When the session was last used, in ms since the epoch (for the idle timeout). */
  lastSeenAt: number;
  user: { sub: string; name: string; email?: string; roles: string[]; personId?: string };
}

/** The display name of the person behind a set of ID-token claims. */
export function displayName(c: IdTokenClaims): string {
  return c.name || c.preferred_username || c.email || c.sub;
}

/** What a new session is made from. */
export interface NewSession {
  accessToken: string;
  refreshToken?: string;
  idToken?: string;
  expiresInSeconds?: number;
  sessionSeconds: number;
  claims: IdTokenClaims;
}

/** Roles from the `roles` claim, else Keycloak's `realm_access.roles`. */
export function rolesOf(c: IdTokenClaims): string[] {
  if (Array.isArray(c.roles)) return c.roles;
  const r = c.realm_access?.roles;
  return Array.isArray(r) ? r : [];
}

/** A new session as of `t` (ms since the epoch): a random id, the tokens, the person. */
export function buildSession(p: NewSession, t: number): OidcSession {
  return {
    id: randomToken(32),
    accessToken: p.accessToken,
    refreshToken: p.refreshToken,
    idToken: p.idToken,
    accessExpiresAt: t + (p.expiresInSeconds ?? 300) * 1000,
    expiresAt: t + p.sessionSeconds * 1000,
    lastSeenAt: t,
    user: {
      sub: p.claims.sub,
      name: displayName(p.claims),
      email: p.claims.email,
      roles: rolesOf(p.claims),
      personId: typeof p.claims.person_id === "string" ? p.claims.person_id : undefined,
    },
  };
}

export class OidcStore {
  private sessions = new Map<string, OidcSession>();
  private transactions = new Map<string, LoginTransaction>();
  /** Sessions that ended because they sat idle (remembered briefly, to tell the person why). */
  private idleEnded = new Set<string>();
  private idleMs: number;

  constructor(
    private readonly now: () => number = Date.now,
    private readonly maxEntries = 10_000,
    idleSeconds = 0,
  ) {
    this.idleMs = idleSeconds * 1000;
  }

  /** Sign a session out after this many idle seconds; `0` turns the idle timeout off (RG-2). */
  setIdleSeconds(seconds: number): void {
    this.idleMs = Math.max(0, seconds) * 1000;
  }

  /** Whether this session id ended because it was idle; asking forgets it. */
  endedIdle(id: string | null | undefined): boolean {
    return id ? this.idleEnded.delete(id) : false;
  }

  private sweep(): void {
    const t = this.now();
    for (const [k, v] of this.transactions) if (v.expiresAt <= t) this.transactions.delete(k);
    for (const [k, v] of this.sessions) if (v.expiresAt <= t) this.sessions.delete(k);
    for (const map of [this.sessions, this.transactions] as Map<string, unknown>[]) {
      // Bounded memory: drop the oldest (insertion order) beyond the cap.
      while (map.size > this.maxEntries) map.delete(map.keys().next().value as string);
    }
  }

  /** Remember a login in flight under its `state`. */
  startTransaction(state: string, tx: Omit<LoginTransaction, "expiresAt">): void {
    this.transactions.set(state, { ...tx, expiresAt: this.now() + TX_TTL_MS });
    this.sweep(); // after the insert, so the cap holds exactly
  }

  /** Take (and forget) the transaction for `state`; `null` if unknown or expired. */
  takeTransaction(state: string): LoginTransaction | null {
    const tx = this.transactions.get(state);
    this.transactions.delete(state);
    if (!tx || tx.expiresAt <= this.now()) return null;
    return tx;
  }

  /** Create a session; returns it (its `id` goes in the cookie). */
  createSession(p: NewSession): OidcSession {
    const session = buildSession(p, this.now());
    this.sessions.set(session.id, session);
    this.sweep(); // after the insert, so the cap holds exactly
    return session;
  }

  /** The live session for a cookie value, or `null`. */
  get(id: string | null | undefined): OidcSession | null {
    if (!id) return null;
    const s = this.sessions.get(id);
    if (!s) return null;
    const t = this.now();
    if (s.expiresAt <= t) {
      this.sessions.delete(id);
      return null;
    }
    if (this.idleMs > 0 && t - s.lastSeenAt >= this.idleMs) {
      this.sessions.delete(id);
      this.idleEnded.add(id);
      while (this.idleEnded.size > 1000) this.idleEnded.delete(this.idleEnded.values().next().value as string);
      return null;
    }
    s.lastSeenAt = t;
    return s;
  }

  delete(id: string | null | undefined): void {
    if (id) this.sessions.delete(id);
  }

  get size(): number {
    return this.sessions.size;
  }
}

/** The process-wide store. */
export const oidcStore = new OidcStore();

/** Refresh a little before the access token expires. */
const REFRESH_MARGIN_MS = 30_000;

/**
 * A usable access token for the session, refreshing it when it is about to
 * expire. A session that cannot be refreshed is ended and `null` is returned,
 * so the caller treats the person as signed out.
 */
export async function accessTokenFor(
  store: SessionStore,
  session: OidcSession,
  deps: {
    fetchFn: FetchFn;
    cfg: OidcConfig;
    meta: ProviderMetadata;
    now?: () => number;
  },
): Promise<string | null> {
  const now = (deps.now ?? Date.now)();
  if (session.accessExpiresAt - REFRESH_MARGIN_MS > now) return session.accessToken;
  if (!session.refreshToken) {
    await store.delete(session.id);
    return null;
  }
  try {
    const t = await refreshTokens(deps.fetchFn, deps.meta, deps.cfg, session.refreshToken);
    session.accessToken = t.access_token;
    if (t.refresh_token) session.refreshToken = t.refresh_token;
    session.accessExpiresAt = now + (t.expires_in ?? 300) * 1000;
    // A shared store holds its own copy: write the refreshed tokens back.
    await store.save(session);
    return session.accessToken;
  } catch {
    await store.delete(session.id);
    return null;
  }
}

/** Provider metadata and keys, cached for ten minutes (keys re-read on demand). */
const CACHE_MS = 10 * 60 * 1000;
let metaCache: { issuer: string; at: number; meta: ProviderMetadata } | null = null;
let keyCache: { uri: string; at: number; keys: Jwk[] } | null = null;

export async function providerMetadata(fetchFn: FetchFn, issuer: string, now = Date.now()): Promise<ProviderMetadata> {
  if (metaCache && metaCache.issuer === issuer && now - metaCache.at < CACHE_MS) {
    return metaCache.meta;
  }
  const meta = await discover(fetchFn, issuer);
  metaCache = { issuer, at: now, meta };
  return meta;
}

/** The signing keys; `refresh` forces a re-read (a key rotated since the cache). */
export async function signingKeys(
  fetchFn: FetchFn,
  meta: ProviderMetadata,
  refresh = false,
  now = Date.now(),
): Promise<Jwk[]> {
  if (!refresh && keyCache && keyCache.uri === meta.jwks_uri && now - keyCache.at < CACHE_MS) {
    return keyCache.keys;
  }
  const keys = await fetchJwks(fetchFn, meta);
  keyCache = { uri: meta.jwks_uri, at: now, keys };
  return keys;
}

/** For tests: forget the caches. */
export function resetOidcCaches(): void {
  metaCache = null;
  keyCache = null;
}

export { OidcError };

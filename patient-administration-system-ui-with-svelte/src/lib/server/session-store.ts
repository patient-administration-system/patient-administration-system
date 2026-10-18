// Where OpenID Connect sessions live (RG-12). Two stores behind one asynchronous interface:
//
// - `MemorySessionStore`: in this process. The default. One front-end replica, or several with
//   sticky sessions; a restart signs everyone out.
// - `RedisSessionStore`: in Redis, so any replica can serve any request, a restart signs nobody
//   out, and a session ends everywhere at once. Sessions hold the identity provider's tokens,
//   so use `rediss://` (TLS) and a password, and keep Redis off the public network.
//
// Which one is chosen by `OIDC_SESSION_STORE` (`memory` or `redis`) and `REDIS_URL`; see
// `session-store-instance.ts`.

import { OidcStore, buildSession, type LoginTransaction, type NewSession, type OidcSession, TX_TTL_MS } from "./oidc-session";
import { RedisClient } from "./resp";

export interface SessionStore {
  startTransaction(state: string, tx: Omit<LoginTransaction, "expiresAt">): Promise<void>;
  /** Take (and spend) the login transaction for `state`; `null` if unknown, spent or expired. */
  takeTransaction(state: string): Promise<LoginTransaction | null>;
  createSession(p: NewSession): Promise<OidcSession>;
  /** The live session for a cookie value (and mark it used), or `null`. */
  get(id: string | null | undefined): Promise<OidcSession | null>;
  /** Write a changed session back (refreshed tokens). */
  save(session: OidcSession): Promise<void>;
  delete(id: string | null | undefined): Promise<void>;
  /** Whether this session id ended because it was idle; asking forgets it. */
  endedIdle(id: string | null | undefined): Promise<boolean>;
  /** Idle seconds before sign-out; `0` turns the idle timeout off. */
  setIdleSeconds(seconds: number): void;
}

/** The in-process store, as the asynchronous interface. */
export class MemorySessionStore implements SessionStore {
  constructor(private readonly inner: OidcStore = new OidcStore()) {}
  async startTransaction(state: string, tx: Omit<LoginTransaction, "expiresAt">): Promise<void> {
    this.inner.startTransaction(state, tx);
  }
  async takeTransaction(state: string): Promise<LoginTransaction | null> {
    return this.inner.takeTransaction(state);
  }
  async createSession(p: NewSession): Promise<OidcSession> {
    return this.inner.createSession(p);
  }
  async get(id: string | null | undefined): Promise<OidcSession | null> {
    return this.inner.get(id);
  }
  async save(): Promise<void> {
    // The session object is the stored one.
  }
  async delete(id: string | null | undefined): Promise<void> {
    this.inner.delete(id);
  }
  async endedIdle(id: string | null | undefined): Promise<boolean> {
    return this.inner.endedIdle(id);
  }
  setIdleSeconds(seconds: number): void {
    this.inner.setIdleSeconds(seconds);
  }
}

/** Writes to Redis for "used" at most this often per session (the idle timeout is not that fine). */
const TOUCH_EVERY_MS = 10_000;
const KEY = { session: "pas:sess:", tx: "pas:tx:", idle: "pas:idle:", rate: "pas:txrate:" } as const;

/** Random identifiers made by `randomToken` (base64url). Anything else is not one of ours. */
const TOKEN = /^[A-Za-z0-9_-]{20,128}$/;

/**
 * Login transactions an unauthenticated visitor may start per minute, for the whole deployment. The
 * in-memory store caps its own size; Redis does not, so without this a loop on `/signin/oidc` could
 * fill it (each key lives ten minutes). Generous for real sign-ins (a morning rush of hundreds).
 */
export const MAX_LOGINS_PER_MINUTE = 600;

function parse<T>(raw: string): T | null {
  try {
    const v = JSON.parse(raw) as unknown;
    return v && typeof v === "object" ? (v as T) : null;
  } catch {
    return null;
  }
}

/** Sessions in Redis. */
export class RedisSessionStore implements SessionStore {
  private idleMs = 0;

  constructor(
    private readonly redis: Pick<RedisClient, "command">,
    private readonly now: () => number = Date.now,
  ) {}

  setIdleSeconds(seconds: number): void {
    this.idleMs = Math.max(0, seconds) * 1000;
  }

  async startTransaction(state: string, tx: Omit<LoginTransaction, "expiresAt">): Promise<void> {
    if (!TOKEN.test(state)) throw new Error("not a login state");
    const minute = Math.floor(this.now() / 60_000);
    const n = await this.redis.command("INCR", KEY.rate + minute);
    if (n === 1) await this.redis.command("EXPIRE", KEY.rate + minute, 120);
    if (typeof n === "number" && n > MAX_LOGINS_PER_MINUTE) throw new Error("too many sign-ins started; try again in a minute");
    const full: LoginTransaction = { ...tx, expiresAt: this.now() + TX_TTL_MS };
    await this.redis.command("SET", KEY.tx + state, JSON.stringify(full), "EX", Math.ceil(TX_TTL_MS / 1000));
  }

  async takeTransaction(state: string): Promise<LoginTransaction | null> {
    if (!TOKEN.test(state)) return null;
    // GETDEL is atomic: two callbacks racing on one state cannot both win.
    const raw = await this.redis.command("GETDEL", KEY.tx + state);
    if (typeof raw !== "string") return null;
    const tx = parse<LoginTransaction>(raw);
    return tx && tx.expiresAt > this.now() ? tx : null;
  }

  async createSession(p: NewSession): Promise<OidcSession> {
    const session = buildSession(p, this.now());
    await this.redis.command("SET", KEY.session + session.id, JSON.stringify(session), "EX", p.sessionSeconds);
    return session;
  }

  async get(id: string | null | undefined): Promise<OidcSession | null> {
    if (!id || !TOKEN.test(id)) return null;
    let raw;
    try {
      raw = await this.redis.command("GET", KEY.session + id);
    } catch (e) {
      // Redis unreachable: treat the person as signed out rather than failing the page.
      console.error(`session store unavailable: ${(e as Error).message}`);
      return null;
    }
    if (typeof raw !== "string") return null;
    const s = parse<OidcSession>(raw);
    if (!s || typeof s.expiresAt !== "number" || typeof s.accessToken !== "string") {
      // Not a session we wrote: do not trust it, and do not keep it.
      await this.delete(id);
      return null;
    }
    const t = this.now();
    if (s.expiresAt <= t) {
      await this.delete(id);
      return null;
    }
    if (this.idleMs > 0 && t - s.lastSeenAt >= this.idleMs) {
      await this.delete(id);
      await this.redis.command("SET", KEY.idle + id, "1", "EX", 3600);
      return null;
    }
    if (t - s.lastSeenAt >= TOUCH_EVERY_MS) {
      s.lastSeenAt = t;
      await this.redis.command("SET", KEY.session + id, JSON.stringify(s), "XX", "KEEPTTL");
    }
    return s;
  }

  async save(session: OidcSession): Promise<void> {
    if (!TOKEN.test(session.id)) throw new Error("not a session id");
    await this.redis.command("SET", KEY.session + session.id, JSON.stringify(session), "XX", "KEEPTTL");
  }

  async delete(id: string | null | undefined): Promise<void> {
    if (id) await this.redis.command("DEL", KEY.session + id);
  }

  async endedIdle(id: string | null | undefined): Promise<boolean> {
    if (!id || !TOKEN.test(id)) return false;
    try {
      return typeof (await this.redis.command("GETDEL", KEY.idle + id)) === "string";
    } catch {
      return false;
    }
  }
}

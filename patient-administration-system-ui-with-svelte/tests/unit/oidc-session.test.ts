// @vitest-environment node
// Sessions and login transactions in memory, token refresh, and the provider caches (QUAL-19).
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  OidcStore,
  TX_TTL_MS,
  accessTokenFor,
  buildSession,
  displayName,
  providerMetadata,
  resetOidcCaches,
  rolesOf,
  signingKeys,
  type OidcSession,
} from "#lib/server/oidc-session.js";
import { MemorySessionStore, type SessionStore } from "#lib/server/session-store.js";
import type { IdTokenClaims, OidcConfig, ProviderMetadata } from "#lib/server/oidc.js";

const CLAIMS: IdTokenClaims = { iss: "i", sub: "kc-1", aud: "a", exp: 0, preferred_username: "ada", realm_access: { roles: ["staff"] } };
const NEW = { accessToken: "at", refreshToken: "rt", sessionSeconds: 3600, claims: CLAIMS };
const TX = { nonce: "n", verifier: "v", returnTo: "/", redirectUri: "r", locale: "en-001" };

function clocked(idleSeconds = 0, maxEntries = 10_000) {
  const clock = { t: 1_800_000_000_000 };
  return { clock, store: new OidcStore(() => clock.t, maxEntries, idleSeconds) };
}

describe("claims", () => {
  it("take roles from `roles`, else Keycloak's realm roles, else none", () => {
    expect(rolesOf({ ...CLAIMS, roles: ["admin"] })).toEqual(["admin"]);
    expect(rolesOf(CLAIMS)).toEqual(["staff"]);
    expect(rolesOf({ ...CLAIMS, realm_access: undefined })).toEqual([]);
  });

  it("name a person by name, then username, email, subject", () => {
    expect(displayName({ ...CLAIMS, name: "Ada Lovelace" })).toBe("Ada Lovelace");
    expect(displayName(CLAIMS)).toBe("ada");
    expect(displayName({ ...CLAIMS, preferred_username: undefined, email: "a@x" })).toBe("a@x");
    expect(displayName({ ...CLAIMS, preferred_username: undefined })).toBe("kc-1");
  });

  it("keep a person id only when it is a string", () => {
    expect(buildSession({ ...NEW, claims: { ...CLAIMS, person_id: "p-1" } }, 0).user.personId).toBe("p-1");
    expect(buildSession({ ...NEW, claims: { ...CLAIMS, person_id: 7 as never } }, 0).user.personId).toBeUndefined();
  });
});

describe("OidcStore", () => {
  it("spends a login transaction once, and not after its ten minutes", () => {
    const { clock, store } = clocked();
    store.startTransaction("s", TX);
    expect(store.takeTransaction("s")).toMatchObject({ nonce: "n" });
    expect(store.takeTransaction("s")).toBeNull();
    store.startTransaction("s", TX);
    clock.t += TX_TTL_MS;
    expect(store.takeTransaction("s")).toBeNull();
  });

  it("ends a session at its hard end", () => {
    const { clock, store } = clocked();
    const s = store.createSession(NEW);
    expect(store.get(s.id)?.user.roles).toEqual(["staff"]);
    clock.t += 3600 * 1000;
    expect(store.get(s.id)).toBeNull();
    expect(store.get(null)).toBeNull();
  });

  it("ends an idle session, remembers why once, and use keeps it alive", () => {
    const { clock, store } = clocked(60);
    const s = store.createSession(NEW);
    clock.t += 59_000;
    expect(store.get(s.id)).not.toBeNull();
    clock.t += 59_000;
    expect(store.get(s.id)).not.toBeNull();
    clock.t += 60_000;
    expect(store.get(s.id)).toBeNull();
    expect(store.endedIdle(s.id)).toBe(true);
    expect(store.endedIdle(s.id)).toBe(false);
  });

  it("keeps memory bounded by dropping the oldest", () => {
    const { store } = clocked(0, 3);
    const ids = Array.from({ length: 5 }, () => store.createSession(NEW).id);
    expect(store.size).toBe(3);
    expect(store.get(ids[0])).toBeNull();
    expect(store.get(ids[4])).not.toBeNull();
  });

  it("deletes a session at once", () => {
    const { store } = clocked();
    const s = store.createSession(NEW);
    store.delete(s.id);
    expect(store.get(s.id)).toBeNull();
  });
});

const META: ProviderMetadata = { issuer: "https://idp", authorization_endpoint: "a", token_endpoint: "https://idp/token", jwks_uri: "https://idp/keys" };
const CFG: OidcConfig = { issuer: "https://idp", clientId: "c", scopes: "openid" };

describe("accessTokenFor", () => {
  let mem: OidcStore;
  let store: SessionStore;
  let clock: { t: number };
  beforeEach(() => {
    ({ clock, store: mem } = clocked());
    store = new MemorySessionStore(mem);
  });
  const deps = (fetchFn: typeof fetch) => ({ fetchFn, cfg: CFG, meta: META, now: () => clock.t });

  it("uses a token that is not about to expire, without asking the provider", async () => {
    const s = mem.createSession({ ...NEW, expiresInSeconds: 300 });
    const f = vi.fn() as unknown as typeof fetch;
    await expect(accessTokenFor(store, s, deps(f))).resolves.toBe("at");
    expect(f).not.toHaveBeenCalled();
  });

  it("refreshes one that is, and keeps the new tokens", async () => {
    const s = mem.createSession({ ...NEW, expiresInSeconds: 20 });
    const save = vi.spyOn(store, "save");
    const f = vi.fn(async () => new Response(JSON.stringify({ access_token: "at2", refresh_token: "rt2", expires_in: 600 }))) as unknown as typeof fetch;
    await expect(accessTokenFor(store, s, deps(f))).resolves.toBe("at2");
    expect(s.refreshToken).toBe("rt2");
    expect(s.accessExpiresAt).toBe(clock.t + 600_000);
    expect(save).toHaveBeenCalledWith(s);
  });

  it("signs the person out when it cannot refresh", async () => {
    const noRefresh = mem.createSession({ ...NEW, refreshToken: undefined, expiresInSeconds: 0 });
    await expect(accessTokenFor(store, noRefresh, deps(vi.fn() as unknown as typeof fetch))).resolves.toBeNull();
    expect(mem.get(noRefresh.id)).toBeNull();

    const refused = mem.createSession({ ...NEW, expiresInSeconds: 0 });
    const f = vi.fn(async () => new Response("{}", { status: 400 })) as unknown as typeof fetch;
    await expect(accessTokenFor(store, refused, deps(f))).resolves.toBeNull();
    expect(mem.get(refused.id)).toBeNull();
  });
});

describe("provider caches", () => {
  beforeEach(() => resetOidcCaches());

  it("read the metadata once per ten minutes per issuer", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify(META))) as unknown as typeof fetch;
    await providerMetadata(f, "https://idp", 0);
    await providerMetadata(f, "https://idp", 9 * 60_000);
    expect(f).toHaveBeenCalledTimes(1);
    await providerMetadata(f, "https://idp", 10 * 60_000);
    expect(f).toHaveBeenCalledTimes(2);
  });

  it("re-read the keys on demand, for a key rotated since", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ keys: [] }))) as unknown as typeof fetch;
    await signingKeys(f, META, false, 0);
    await signingKeys(f, META, false, 1000);
    expect(f).toHaveBeenCalledTimes(1);
    await signingKeys(f, META, true, 2000);
    expect(f).toHaveBeenCalledTimes(2);
  });
});

// The memory store as the asynchronous interface the hooks use.
describe("MemorySessionStore", () => {
  it("passes through to the in-process store", async () => {
    const { store: mem } = clocked(60);
    const store = new MemorySessionStore(mem);
    store.setIdleSeconds(0);
    await store.startTransaction("s", TX);
    await expect(store.takeTransaction("s")).resolves.toMatchObject({ verifier: "v" });
    const s: OidcSession = await store.createSession(NEW);
    await expect(store.get(s.id)).resolves.toBe(s);
    await store.save(s);
    await expect(store.endedIdle(s.id)).resolves.toBe(false);
    await store.delete(s.id);
    await expect(store.get(s.id)).resolves.toBeNull();
  });
});

// @vitest-environment node
// Server configuration read from the environment (QUAL-19): the service token (a rotating file is re-read,
// an unreadable one gives no credential), the OIDC settings and their defaults, and which session store
// is chosen. Each test loads the modules afresh against its own environment.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** Every variable these modules read, unset unless a test sets it (the mock refuses unknown names). */
const NAMES = [
  "PAS_API_URL", "PAS_API_TOKEN", "PAS_API_TOKEN_FILE", "OIDC_ISSUER", "OIDC_CLIENT_ID", "OIDC_CLIENT_SECRET",
  "OIDC_SCOPES", "OIDC_TENANT_ID", "OIDC_IDP_HINT", "OIDC_REDIRECT_URI", "OIDC_SESSION_HOURS",
  "OIDC_IDLE_MINUTES", "OIDC_END_SESSION", "OIDC_SESSION_STORE", "REDIS_URL",
];

async function withEnv(env: Record<string, string>) {
  vi.resetModules();
  vi.doMock("$app/env/private", () => ({ ...Object.fromEntries(NAMES.map((n) => [n, undefined])), ...env }));
  return {
    config: await import("#lib/server/config.js"),
    instance: await import("#lib/server/session-store-instance.js"),
    stores: await import("#lib/server/session-store.js"),
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.doUnmock("$app/env/private");
  vi.restoreAllMocks();
});

describe("serviceToken", () => {
  it("is PAS_API_TOKEN when there is no token file, and empty when neither is set", async () => {
    expect((await withEnv({ PAS_API_TOKEN: "t1" })).config.serviceToken()).toBe("t1");
    expect((await withEnv({})).config.serviceToken()).toBe("");
  });

  it("reads a token file, re-reads it after five seconds, and gives nothing for an unreadable one", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pas-token-"));
    const file = join(dir, "token");
    try {
      writeFileSync(file, "first\n");
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(1_800_000_000_000);
      const { config } = await withEnv({ PAS_API_TOKEN: "ignored", PAS_API_TOKEN_FILE: file });
      expect(config.serviceToken()).toBe("first");
      writeFileSync(file, "second");
      vi.setSystemTime(1_800_000_004_000);
      expect(config.serviceToken()).toBe("first");
      vi.setSystemTime(1_800_000_005_000);
      expect(config.serviceToken()).toBe("second");
      rmSync(file);
      vi.setSystemTime(1_800_000_011_000);
      expect(config.serviceToken()).toBe("");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("OIDC settings", () => {
  it("need both an issuer and a client id", async () => {
    expect((await withEnv({ OIDC_ISSUER: "https://idp" })).config.oidcConfig()).toBeNull();
    expect((await withEnv({ OIDC_CLIENT_ID: "c" })).config.oidcConfig()).toBeNull();
  });

  it("default the scopes and leave unset options out", async () => {
    const { config } = await withEnv({ OIDC_ISSUER: " https://idp ", OIDC_CLIENT_ID: "c", OIDC_CLIENT_SECRET: " " });
    expect(config.oidcConfig()).toEqual({
      issuer: "https://idp",
      clientId: "c",
      clientSecret: undefined,
      scopes: "openid profile email",
      tenantId: undefined,
      idpHint: undefined,
    });
  });

  it("default to an 8-hour session and 30 idle minutes, and refuse nonsense", async () => {
    expect((await withEnv({})).config.oidcOptions()).toEqual({
      redirectUri: undefined,
      sessionSeconds: 8 * 3600,
      idleSeconds: 30 * 60,
      endProviderSession: false,
    });
    const o = (await withEnv({ OIDC_SESSION_HOURS: "-1", OIDC_IDLE_MINUTES: "abc", OIDC_END_SESSION: "true" })).config.oidcOptions();
    expect([o.sessionSeconds, o.idleSeconds, o.endProviderSession]).toEqual([8 * 3600, 30 * 60, true]);
    const z = (await withEnv({ OIDC_SESSION_HOURS: "0.5", OIDC_IDLE_MINUTES: "0" })).config.oidcOptions();
    expect([z.sessionSeconds, z.idleSeconds]).toEqual([1800, 0]);
  });
});

describe("the session store", () => {
  it("is in memory by default, and the same one every time", async () => {
    const { instance, stores } = await withEnv({});
    expect(instance.sessions()).toBeInstanceOf(stores.MemorySessionStore);
    expect(instance.sessions()).toBe(instance.sessions());
  });

  it("is Redis when asked, and warns about a remote server without TLS", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const local = await withEnv({ OIDC_SESSION_STORE: "Redis", REDIS_URL: "redis://127.0.0.1:6379" });
    expect(local.instance.sessions()).toBeInstanceOf(local.stores.RedisSessionStore);
    expect(warn).not.toHaveBeenCalled();
    await (await withEnv({ OIDC_SESSION_STORE: "redis", REDIS_URL: "rediss://cache.internal:6380" })).instance.sessions();
    expect(warn).not.toHaveBeenCalled();
    await (await withEnv({ OIDC_SESSION_STORE: "redis", REDIS_URL: "redis://cache.internal:6379" })).instance.sessions();
    expect(warn).toHaveBeenCalledOnce();
  });

  it("stops the server on a misconfiguration", async () => {
    expect((await withEnv({ OIDC_SESSION_STORE: "redis" })).instance.sessions).toThrow(/REDIS_URL/);
    expect((await withEnv({ OIDC_SESSION_STORE: "postgres" })).instance.sessions).toThrow(/memory or redis/);
  });
});

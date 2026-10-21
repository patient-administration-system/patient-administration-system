// @vitest-environment node
// The Redis session store (QUAL-19), against an in-memory fake of the few Redis commands it uses and a
// controllable clock: a login transaction works once, sessions end when they should, an idle sign-out is
// remembered once, a value we did not write is distrusted, and Redis being down signs people out.
import { describe, expect, it } from "vitest";
import { MAX_LOGINS_PER_MINUTE, RedisSessionStore } from "#lib/server/session-store.js";
import type { Resp } from "#lib/server/resp.js";

class FakeRedis {
  data = new Map<string, { value: string; expiresAt: number | null }>();
  down = false;
  constructor(private readonly clock: { t: number }) {}

  private live(k: string) {
    const e = this.data.get(k);
    if (e && e.expiresAt !== null && e.expiresAt <= this.clock.t) this.data.delete(k);
    return this.data.get(k);
  }

  async command(...a: (string | number)[]): Promise<Resp> {
    if (this.down) throw new Error("Redis connection closed");
    const [cmd, key, ...rest] = a.map(String);
    switch (cmd) {
      case "GET":
        return this.live(key!)?.value ?? null;
      case "GETDEL": {
        const v = this.live(key!)?.value ?? null;
        this.data.delete(key!);
        return v;
      }
      case "DEL":
        return this.data.delete(key!) ? 1 : 0;
      case "INCR": {
        const n = Number(this.live(key!)?.value ?? 0) + 1;
        this.data.set(key!, { value: String(n), expiresAt: this.live(key!)?.expiresAt ?? null });
        return n;
      }
      case "EXPIRE": {
        const e = this.live(key!);
        if (e) e.expiresAt = this.clock.t + Number(rest[0]) * 1000;
        return e ? 1 : 0;
      }
      case "SET": {
        const [value, ...opts] = rest;
        const prev = this.live(key!);
        if (opts.includes("XX") && !prev) return null;
        const ex = opts.indexOf("EX");
        const expiresAt =
          ex >= 0
            ? this.clock.t + Number(opts[ex + 1]) * 1000
            : opts.includes("KEEPTTL")
              ? (prev?.expiresAt ?? null)
              : null;
        this.data.set(key!, { value: value!, expiresAt });
        return "OK";
      }
      default:
        throw new Error(`fake Redis has no ${cmd}`);
    }
  }
}

function setup() {
  const clock = { t: 1_800_000_000_000 };
  const redis = new FakeRedis(clock);
  const store = new RedisSessionStore(redis, () => clock.t);
  return { clock, redis, store };
}

const STATE = "s".repeat(32);
const TX = { nonce: "n", verifier: "v", returnTo: "/", redirectUri: "http://ui/cb", locale: "en-001" };
const NEW = {
  accessToken: "access",
  sessionSeconds: 3600,
  claims: { sub: "kc-1", name: "Ada", roles: ["staff"] },
} as unknown as Parameters<RedisSessionStore["createSession"]>[0];

describe("login transactions", () => {
  it("work once, and not after ten minutes", async () => {
    const { clock, store } = setup();
    await store.startTransaction(STATE, TX);
    await expect(store.takeTransaction(STATE)).resolves.toMatchObject({ nonce: "n" });
    await expect(store.takeTransaction(STATE)).resolves.toBeNull();

    await store.startTransaction(STATE, TX);
    clock.t += 10 * 60 * 1000 + 1;
    await expect(store.takeTransaction(STATE)).resolves.toBeNull();
  });

  it("refuse a state that is not one of ours", async () => {
    const { store } = setup();
    await expect(store.startTransaction("short", TX)).rejects.toThrow();
    await expect(store.takeTransaction("pas:sess:../x")).resolves.toBeNull();
  });

  it("are rate limited per minute for the whole deployment", async () => {
    const { clock, store } = setup();
    for (let i = 0; i < MAX_LOGINS_PER_MINUTE; i++) await store.startTransaction(STATE, TX);
    await expect(store.startTransaction(STATE, TX)).rejects.toThrow(/too many/);
    clock.t += 60_000;
    await expect(store.startTransaction(STATE, TX)).resolves.toBeUndefined();
  });
});

describe("sessions", () => {
  it("are found by id until their hard end", async () => {
    const { clock, store } = setup();
    const s = await store.createSession(NEW);
    await expect(store.get(s.id)).resolves.toMatchObject({ user: { sub: "kc-1", roles: ["staff"] } });
    clock.t += 3600 * 1000;
    await expect(store.get(s.id)).resolves.toBeNull();
  });

  it("end when idle, and say so exactly once", async () => {
    const { clock, store } = setup();
    store.setIdleSeconds(30 * 60);
    const s = await store.createSession({ ...NEW, sessionSeconds: 8 * 3600 });
    clock.t += 29 * 60 * 1000;
    await expect(store.get(s.id)).resolves.not.toBeNull(); // use moves the idle clock on
    clock.t += 29 * 60 * 1000;
    await expect(store.get(s.id)).resolves.not.toBeNull();
    clock.t += 30 * 60 * 1000;
    await expect(store.get(s.id)).resolves.toBeNull();
    await expect(store.endedIdle(s.id)).resolves.toBe(true);
    await expect(store.endedIdle(s.id)).resolves.toBe(false);
  });

  it("distrust and remove a value the store did not write", async () => {
    const { redis, store } = setup();
    const id = "x".repeat(32);
    redis.data.set(`pas:sess:${id}`, { value: JSON.stringify({ user: { roles: ["admin"] } }), expiresAt: null });
    await expect(store.get(id)).resolves.toBeNull();
    expect(redis.data.has(`pas:sess:${id}`)).toBe(false);
  });

  it("ignore an id that is not one of ours, without asking Redis", async () => {
    const { redis, store } = setup();
    redis.down = true;
    await expect(store.get("../../etc")).resolves.toBeNull();
    await expect(store.get(undefined)).resolves.toBeNull();
  });

  it("treat Redis being down as signed out, not as a broken page", async () => {
    const { redis, store } = setup();
    const s = await store.createSession(NEW);
    redis.down = true;
    const err = console.error;
    console.error = () => {};
    try {
      await expect(store.get(s.id)).resolves.toBeNull();
      await expect(store.endedIdle(s.id)).resolves.toBe(false);
    } finally {
      console.error = err;
    }
  });

  it("delete ends a session at once", async () => {
    const { store } = setup();
    const s = await store.createSession(NEW);
    await store.delete(s.id);
    await expect(store.get(s.id)).resolves.toBeNull();
  });
});

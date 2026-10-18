// The session store this process uses, chosen from the environment (RG-12).

import * as env from "$app/env/private";
import { RedisClient } from "./resp";
import { MemorySessionStore, RedisSessionStore, type SessionStore } from "./session-store";

let chosen: SessionStore | null = null;

/** `memory` (default) or `redis` with `REDIS_URL`. A misconfiguration stops the server at start. */
export function sessions(): SessionStore {
  if (chosen) return chosen;
  const kind = (env.OIDC_SESSION_STORE ?? "").trim().toLowerCase() || "memory";
  if (kind === "memory") {
    chosen = new MemorySessionStore();
  } else if (kind === "redis") {
    const url = (env.REDIS_URL ?? "").trim();
    if (!url) throw new Error("OIDC_SESSION_STORE=redis needs REDIS_URL");
    if (url.startsWith("redis://") && !/@(localhost|127\.0\.0\.1|\[::1\])([:/]|$)|\/\/(localhost|127\.0\.0\.1|\[::1\])([:/]|$)/.test(url)) {
      console.warn("REDIS_URL is not TLS (redis://) and not local: sessions hold identity-provider tokens; prefer rediss://");
    }
    chosen = new RedisSessionStore(new RedisClient({ url }));
  } else {
    throw new Error(`OIDC_SESSION_STORE must be memory or redis, not ${JSON.stringify(kind)}`);
  }
  return chosen;
}

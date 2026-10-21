// The shared route helpers (QUAL-21): what a PAS failure becomes on a page, in a form, and on a token link.
import { describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({ PAS_API_URL: "http://pas.test", PAS_API_TOKEN: "t", PAS_API_TOKEN_FILE: "" }));

const { PasError } = await import("#lib/server/pas.js");
const { actorName, fromPas, pasFailure, signedInFetch } = await import("#lib/server/route.js");
const { LINK_PAGE_HEADERS, isLinkToken, linkFailureStatus, linkStateFor } = await import("#lib/server/token-link.js");

const locals = (user: { sub: string; name?: string } | null) =>
  ({ oidc: user ? { user: { roles: [], name: "", ...user } } : null }) as Parameters<typeof actorName>[0];

async function thrown(p: Promise<unknown>): Promise<{ status: number; body?: { message: string } }> {
  try {
    await p;
  } catch (e) {
    return e as { status: number };
  }
  throw new Error("no error");
}

describe("fromPas", () => {
  it("passes the data through", async () => {
    await expect(fromPas(Promise.resolve([1]))).resolves.toEqual([1]);
  });

  it("turns a PAS failure into a 502 page, and a 404 into 404 only when asked", async () => {
    expect((await thrown(fromPas(Promise.reject(new PasError(500, "the PAS API answered 500"))))).status).toBe(502);
    expect((await thrown(fromPas(Promise.reject(new PasError(404, "x"))))).status).toBe(502);
    expect((await thrown(fromPas(Promise.reject(new PasError(404, "x")), { notFound: true }))).status).toBe(404);
  });

  it("lets anything else through as the bug it is", async () => {
    await expect(fromPas(Promise.reject(new TypeError("bug")))).rejects.toThrow(TypeError);
  });
});

describe("pasFailure", () => {
  it("is null when the call worked", async () => {
    await expect(pasFailure(async () => "ok", { id: "1" })).resolves.toBeNull();
  });

  it("is a 502 form failure when the PAS failed and a 400 when it refused, carrying the form data", async () => {
    await expect(pasFailure(() => Promise.reject(new PasError(503, "x")), { id: "1" })).resolves.toMatchObject({ status: 502, data: { id: "1" } });
    await expect(pasFailure(() => Promise.reject(new PasError(422, "x")), { id: "1" })).resolves.toMatchObject({ status: 400, data: { id: "1" } });
  });

  it("throws anything that is not a PAS failure", async () => {
    await expect(pasFailure(() => Promise.reject(new RangeError("bug")), {})).rejects.toThrow(RangeError);
  });
});

describe("the signed-in person", () => {
  it("is named by name, else subject, at most 120 characters, and never by the form", () => {
    expect(actorName(locals({ sub: "kc-1", name: "Ada" }))).toBe("Ada");
    expect(actorName(locals({ sub: "kc-1", name: undefined }))).toBe("kc-1");
    expect(actorName(locals({ sub: "kc-1", name: "x".repeat(300) }))).toHaveLength(120);
    expect(actorName(locals(null))).toBe("");
  });

  it("is who the PAS is told a call is for", async () => {
    const f = vi.fn(async (_u: RequestInfo | URL, _i?: RequestInit) => new Response("{}"));
    await signedInFetch(f as unknown as typeof fetch, locals({ sub: "kc-7" }))("http://pas.test/x");
    expect(new Headers(f.mock.calls[0]![1]!.headers).get("x-on-behalf-of")).toBe("kc-7");
  });
});

describe("token links", () => {
  it("accept only a token the PAS could have minted", () => {
    expect(isLinkToken("a".repeat(64))).toBe(true);
    for (const t of ["A".repeat(64), "a".repeat(63), "a".repeat(65), `${"a".repeat(63)}/`, ""]) expect(isLinkToken(t), t).toBe(false);
  });

  it("map a failed PAS call to what the page says, and to the form status", () => {
    expect([404, 409, 400, 500].map((s) => linkStateFor(new PasError(s, "x")))).toEqual(["invalid", "used", "expired", "unavailable"]);
    expect(linkStateFor(new TypeError("fetch failed"))).toBe("unavailable");
    expect([404, 409, 503].map((s) => linkFailureStatus(new PasError(s, "x")))).toEqual([404, 409, 502]);
    expect(linkFailureStatus(new Error("bug"))).toBe(502);
  });

  it("keep the token out of Referer headers and caches", () => {
    expect(LINK_PAGE_HEADERS).toEqual({ "referrer-policy": "same-origin", "cache-control": "no-store" });
  });
});

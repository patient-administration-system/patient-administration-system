import { describe, expect, it, vi } from "vitest";
import { ON_BEHALF_OF, actingAs, sanitizeActor } from "#lib/acting-as.js";

describe("sanitizeActor", () => {
  it("keeps a normal subject and drops what could break a header or an audit line", () => {
    expect(sanitizeActor("3f2c1a9e-0000-4000-8000-000000000001")).toBe("3f2c1a9e-0000-4000-8000-000000000001");
    expect(sanitizeActor("ada\r\nX-Evil: 1")).toBe("adaX-Evil: 1");
    expect(sanitizeActor("  padded  ")).toBe("padded");
    expect(sanitizeActor("日本語")).toBeUndefined();
    expect(sanitizeActor("")).toBeUndefined();
    expect(sanitizeActor(null)).toBeUndefined();
    expect(sanitizeActor(undefined)).toBeUndefined();
  });
  it("caps the length", () => {
    expect(sanitizeActor("x".repeat(500))?.length).toBe(200);
  });
});

describe("actingAs", () => {
  const spy = () => vi.fn(async (_i: RequestInfo | URL, _init?: RequestInit) => new Response("{}"));

  it("adds X-On-Behalf-Of and keeps the caller's other headers and options", async () => {
    const f = spy();
    await actingAs(f as unknown as typeof fetch, "nurse-7")("http://api/x", {
      method: "POST",
      headers: { authorization: "Bearer t", "content-type": "application/json" },
      body: "{}",
    });
    const init = f.mock.calls[0]![1]!;
    const h = new Headers(init.headers);
    expect(h.get(ON_BEHALF_OF)).toBe("nurse-7");
    expect(h.get("authorization")).toBe("Bearer t");
    expect(h.get("content-type")).toBe("application/json");
    expect(init.method).toBe("POST");
    expect(init.body).toBe("{}");
  });

  it("overrides an actor already on the request instead of trusting it", async () => {
    const f = spy();
    await actingAs(f as unknown as typeof fetch, "real-user")("http://api/x", {
      headers: { [ON_BEHALF_OF]: "someone-else" },
    });
    expect(new Headers(f.mock.calls[0]![1]!.headers).get(ON_BEHALF_OF)).toBe("real-user");
  });

  it("returns the original fetch when there is no actor", () => {
    const f = spy() as unknown as typeof fetch;
    expect(actingAs(f, undefined)).toBe(f);
    expect(actingAs(f, "")).toBe(f);
  });
});

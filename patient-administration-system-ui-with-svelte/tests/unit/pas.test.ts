// @vitest-environment node
// The BFF's PAS client (QUAL-19): the bearer token is added on the server, every failure becomes a PasError
// with a status the page translates, and what the PAS said never travels into that error.
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({
  PAS_API_URL: "http://pas.test/",
  PAS_API_TOKEN: "service-token",
  PAS_API_TOKEN_FILE: "",
}));

const { PasError, identityProofed, isCursor, nextCursor, pas, pasArray, pasPage, toleranceStatus, withCursor } =
  await import("#lib/server/pas.js");

type Call = { url: string; init: RequestInit };

function fake(status: number, body: string): { fetch: typeof fetch; calls: Call[] } {
  const calls: Call[] = [];
  const f = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return new Response(body, { status });
  });
  return { fetch: f as unknown as typeof fetch, calls };
}

afterEach(() => vi.restoreAllMocks());

describe("pas()", () => {
  it("calls the configured API with the service token and returns the envelope's data", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ success: true, data: { id: "a1" } }));
    await expect(pas(fetch, "/api/x?y=1")).resolves.toEqual({ id: "a1" });
    expect(calls[0]!.url).toBe("http://pas.test/api/x?y=1");
    const h = calls[0]!.init.headers as Record<string, string>;
    expect(h.authorization).toBe("Bearer service-token");
    expect(h["content-type"]).toBeUndefined();
  });

  it("sends a JSON body with its content type", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ data: {} }));
    await pas(fetch, "/api/x", { method: "PUT", body: { a: 1 } });
    expect(calls[0]!.init.method).toBe("PUT");
    expect(calls[0]!.init.body).toBe('{"a":1}');
    expect((calls[0]!.init.headers as Record<string, string>)["content-type"]).toBe("application/json");
  });

  it("turns an unreachable API into a 502", async () => {
    const f = vi.fn(async () => {
      throw new TypeError("connect ECONNREFUSED 10.0.0.5:8080");
    }) as unknown as typeof fetch;
    const e = await pas(f, "/api/x").catch((x: unknown) => x);
    expect(e).toBeInstanceOf(PasError);
    expect((e as InstanceType<typeof PasError>).status).toBe(502);
    expect((e as Error).message).not.toContain("10.0.0.5");
  });

  it("keeps the API's status and never its words: they go to the server log only", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const secret = 'relation "patients" does not exist at /srv/pas/src/db.rs:42';
    const { fetch } = fake(
      500,
      JSON.stringify({ type: "about:blank", title: "Internal Server Error", status: 500, code: "DB", detail: secret }),
    );
    const e = (await pas(fetch, "/api/x?person=abc").catch((x: unknown) => x)) as InstanceType<typeof PasError>;
    expect(e).toBeInstanceOf(PasError);
    expect(e.status).toBe(500);
    expect(e.message).toBe("the PAS API answered 500");
    expect(log).toHaveBeenCalledOnce();
    expect(String(log.mock.calls[0]![0])).toContain(secret);
    // The query string (which can hold an identifier) is not logged.
    expect(String(log.mock.calls[0]![0])).not.toContain("person=abc");
  });

  it("does not trip over an error that is not JSON", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { fetch } = fake(503, "<html>Service Unavailable</html>");
    await expect(pas(fetch, "/api/x")).rejects.toMatchObject({ status: 503 });
  });
});

describe("pasArray()", () => {
  it("returns a list", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: [1, 2] }));
    await expect(pasArray(fetch, "/api/x")).resolves.toEqual([1, 2]);
  });

  it("treats anything but a list as a broken answer (502)", async () => {
    for (const body of [JSON.stringify({ data: { not: "a list" } }), JSON.stringify({ data: null }), "not json"]) {
      const { fetch } = fake(200, body);
      await expect(pasArray(fetch, "/api/x"), body).rejects.toMatchObject({ status: 502 });
    }
  });
});

describe("pasPage()", () => {
  it("says where a list was cut, and null for a whole one or a body with no meta", async () => {
    const page = (meta: unknown) => pasPage(fake(200, JSON.stringify({ data: [1], meta })).fetch, "/list");
    await expect(page({ limit: 1, returned: 1, truncated: true })).resolves.toEqual({
      rows: [1],
      cutAt: 1,
      next: null,
    });
    await expect(page({ limit: 5, returned: 1, truncated: false })).resolves.toEqual({
      rows: [1],
      cutAt: null,
      next: null,
    });
    await expect(page(undefined)).resolves.toEqual({ rows: [1], cutAt: null, next: null });
  });

  it("gives the cursor of the next page, and asks for a page with the cursor it is given", async () => {
    const meta = { limit: 2, returned: 2, truncated: true, next: "/api/list?open=true&limit=2&cursor=AbC_-9" };
    const { fetch, calls } = fake(200, JSON.stringify({ data: [1, 2], meta }));
    await expect(pasPage(fetch, "/api/list?open=true", "Xy_-1")).resolves.toEqual({
      rows: [1, 2],
      cutAt: 2,
      next: "AbC_-9",
    });
    expect(calls[0]?.url).toBe("http://pas.test/api/list?open=true&cursor=Xy_-1");
  });

  it("never sends a cursor of the wrong shape", async () => {
    for (const bad of ["", "../admin", "a b", "a&limit=500", "x".repeat(257)]) {
      expect(isCursor(bad), bad).toBe(false);
      expect(withCursor("/api/list", bad), bad).toBe("/api/list");
    }
    expect(withCursor("/api/list", null)).toBe("/api/list");
    expect(withCursor("/api/list", "ok_-1")).toBe("/api/list?cursor=ok_-1");
    expect(withCursor("/api/list?a=1", "ok")).toBe("/api/list?a=1&cursor=ok");
  });

  it("has no next cursor for a whole list, or a next without one", () => {
    expect(nextCursor(undefined)).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: false, next: "/x?cursor=a" })).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: true })).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: true, next: "/x?limit=1" })).toBeNull();
  });

  it("treats a body that is not a list as a broken answer", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: {} }));
    await expect(pasPage(fetch, "/list")).rejects.toMatchObject({ status: 502 });
  });
});

describe("toleranceStatus()", () => {
  const good = { overall: "green", from: "a", to: "b", readings: [], by_service: [], by_ward: [] };

  it("returns a status that has its three lists", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: good }));
    await expect(toleranceStatus(fetch, 30)).resolves.toEqual({ ...good, cutAt: null });
  });

  it("says where its lists were cut", async () => {
    const meta = { limit: 500, returned: 500, truncated: true };
    const { fetch } = fake(200, JSON.stringify({ data: good, meta }));
    await expect(toleranceStatus(fetch, 30)).resolves.toEqual({ ...good, cutAt: 500 });
  });

  it("treats a body without them as a broken answer (502), so the page never renders it", async () => {
    for (const data of [null, {}, { ...good, readings: undefined }, { ...good, by_ward: "none" }, [1]]) {
      const { fetch } = fake(200, JSON.stringify({ data }));
      await expect(toleranceStatus(fetch, 30), JSON.stringify(data)).rejects.toMatchObject({ status: 502 });
    }
  });
});

describe("identityProofed()", () => {
  it("is true only for an explicit proofed: true", async () => {
    for (const [data, want] of [
      [{ proofed: true }, true],
      [{ proofed: "true" }, false],
      [{ proofed: 1 }, false],
      [{}, false],
    ] as const) {
      const { fetch, calls } = fake(200, JSON.stringify({ data }));
      await expect(identityProofed(fetch, "kc|sub/1", "p 1"), JSON.stringify(data)).resolves.toBe(want);
      expect(calls[0]!.url).toBe("http://pas.test/api/identity-proofings/kc%7Csub%2F1/check?person_id=p%201");
    }
  });
});

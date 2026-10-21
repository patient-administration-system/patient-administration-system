// @vitest-environment node
// The BFF's PAS client (QUAL-19): the bearer token is added on the server, every failure becomes a PasError
// with a status the page translates, and what the PAS said never travels into that error.
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({ PAS_API_URL: "http://pas.test/", PAS_API_TOKEN: "service-token", PAS_API_TOKEN_FILE: "" }));

const { PasError, identityProofed, pas, pasArray } = await import("#lib/server/pas.js");

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
    const { fetch } = fake(500, JSON.stringify({ success: false, error: { code: "DB", message: secret } }));
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

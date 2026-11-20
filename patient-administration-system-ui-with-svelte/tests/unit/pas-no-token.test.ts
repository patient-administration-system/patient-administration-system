// @vitest-environment node
// The PAS client must also work in deployments that use no service bearer token.
import { describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({
  PAS_API_URL: "http://pas.test/",
  PAS_API_TOKEN: "",
  PAS_API_TOKEN_FILE: "",
}));

const { pas } = await import("#lib/server/pas.js");

describe("pas() without a service token", () => {
  it("omits the authorization header", async () => {
    const fetch = vi.fn(async (_url: RequestInfo | URL, init?: RequestInit) => {
      expect(new Headers(init?.headers).has("authorization")).toBe(false);
      return new Response(JSON.stringify({ data: "ok" }));
    }) as unknown as typeof globalThis.fetch;

    await expect(pas(fetch, "/api/health")).resolves.toBe("ok");
  });
});

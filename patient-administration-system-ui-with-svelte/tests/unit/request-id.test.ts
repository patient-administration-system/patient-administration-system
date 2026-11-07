// @vitest-environment node
// One id per request, carried into the PAS API: the incoming one when it is safe, else a new one, and only on
// requests bound for the API.
import { describe, expect, it } from "vitest";
import { REQUEST_ID, requestIdFor, withRequestId } from "#lib/server/request-id.js";

describe("requestIdFor()", () => {
  it("keeps a safe incoming id", () => {
    expect(requestIdFor("abc-123_x.y")).toBe("abc-123_x.y");
  });

  it("makes a new one for none, or for anything that could put more than an id in a log", () => {
    for (const bad of [null, undefined, "", "a b", "a\nb", "<script>", "x".repeat(129), "é"]) {
      const id = requestIdFor(bad);
      expect(id, String(bad)).toMatch(/^[0-9a-f-]{36}$/);
      expect(id).not.toBe(bad);
    }
  });
});

describe("withRequestId()", () => {
  const api = "http://pas.test";

  it("adds the id to a request for the API", () => {
    const out = withRequestId(new Request(`${api}/api/referrals`), "id-1", api);
    expect(out.headers.get(REQUEST_ID)).toBe("id-1");
  });

  it("keeps the request's method, body and other headers", async () => {
    const out = withRequestId(
      new Request(`${api}/api/x`, { method: "POST", body: "{}", headers: { authorization: "Bearer t" } }),
      "id-2",
      api,
    );
    expect([out.method, out.headers.get("authorization"), await out.text()]).toEqual(["POST", "Bearer t", "{}"]);
  });

  it("leaves a request for anywhere else alone", () => {
    for (const url of [
      "http://keycloak.test/realms/pas",
      "http://pas.test.evil.example/api",
      "http://pas.tester/api",
    ]) {
      expect(withRequestId(new Request(url), "id-3", api).headers.get(REQUEST_ID), url).toBeNull();
    }
  });
});

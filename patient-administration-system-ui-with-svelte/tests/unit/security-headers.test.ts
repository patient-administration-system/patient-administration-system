// NFR-7: the security-header set, applied without clobbering a route's own headers.
import { describe, expect, it } from "vitest";
import { HSTS_VALUE, SECURITY_HEADERS, applySecurityHeaders, isHttps } from "#lib/security-headers.js";

describe("applySecurityHeaders (NFR-7)", () => {
  it("adds the whole set over HTTP, without HSTS", () => {
    const h = applySecurityHeaders(new Headers(), false);
    for (const name of Object.keys(SECURITY_HEADERS)) expect(h.has(name), name).toBe(true);
    expect(h.has("Strict-Transport-Security")).toBe(false);
    expect(h.get("X-Frame-Options")).toBe("DENY");
    expect(h.get("X-Content-Type-Options")).toBe("nosniff");
  });
  it("adds HSTS over HTTPS", () => {
    expect(applySecurityHeaders(new Headers(), true).get("Strict-Transport-Security")).toBe(HSTS_VALUE);
  });
  it("never overwrites a header the route set", () => {
    const h = applySecurityHeaders(new Headers({ "X-Frame-Options": "SAMEORIGIN" }), false);
    expect(h.get("X-Frame-Options")).toBe("SAMEORIGIN");
  });
  it("denies powerful browser features by default", () => {
    const policy = SECURITY_HEADERS["Permissions-Policy"]!;
    for (const f of ["camera", "microphone", "geolocation", "payment"]) expect(policy).toContain(`${f}=()`);
  });
});

describe("isHttps", () => {
  it("trusts the URL, or the first forwarded protocol", () => {
    expect(isHttps(new URL("https://x"), null)).toBe(true);
    expect(isHttps(new URL("http://x"), "https, http")).toBe(true);
    expect(isHttps(new URL("http://x"), "http")).toBe(false);
    expect(isHttps(new URL("http://x"), null)).toBe(false);
  });
});

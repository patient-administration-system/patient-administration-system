// NFR-7: the security-header set, applied without clobbering a route's own headers.
import { describe, expect, it } from "vitest";
import {
  HSTS_VALUE,
  SECURITY_HEADERS,
  allowFormActionTo,
  applySecurityHeaders,
  isHttps,
} from "#lib/security-headers.js";

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

describe("allowFormActionTo (sign-out through the identity provider)", () => {
  const kc = "https://id.example.org";
  it("adds the provider's origin to form-action and touches nothing else", () => {
    expect(allowFormActionTo("default-src 'self'; form-action 'self'; frame-ancestors 'none'", kc)).toBe(
      `default-src 'self'; form-action 'self' ${kc}; frame-ancestors 'none'`,
    );
  });
  it("works when form-action is first or last", () => {
    expect(allowFormActionTo("form-action 'self'", kc)).toBe(`form-action 'self' ${kc}`);
    expect(allowFormActionTo("base-uri 'self';form-action 'self'", kc)).toBe(
      `base-uri 'self';form-action 'self' ${kc}`,
    );
  });
  it("does not add it twice, and leaves a policy without form-action alone", () => {
    const once = allowFormActionTo("form-action 'self'", kc);
    expect(allowFormActionTo(once, kc)).toBe(once);
    expect(allowFormActionTo("default-src 'self'", kc)).toBe("default-src 'self'");
  });
});

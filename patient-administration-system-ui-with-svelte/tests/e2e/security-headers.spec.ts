// NFR-7 in a browser: every response carries the header set and a Content-Security-Policy, and
// rendering a page (Lily widgets included) raises no CSP violation, in several languages.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";

const EXPECTED = [
  "x-content-type-options",
  "x-frame-options",
  "referrer-policy",
  "permissions-policy",
  "cross-origin-opener-policy",
];

test("the sign-in page sends the security headers and a strict CSP", async ({ request }) => {
  const res = await request.get("/en-001/signin");
  for (const h of EXPECTED) expect(res.headers()[h], h).toBeTruthy();
  const csp = res.headers()["content-security-policy"] ?? "";
  expect(csp).toContain("default-src 'self'");
  expect(csp).toContain("frame-ancestors 'none'");
  expect(csp).toContain("object-src 'none'");
  expect(csp).not.toMatch(/script-src[^;]*'unsafe-(inline|eval)'/);
});

for (const locale of ["en-001", "cy-001", "ar-001"]) {
  test(`no CSP violation rendering the sign-in page in ${locale}`, async ({ page }) => {
    const violations: string[] = [];
    await page.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (e) => {
        (window as unknown as { __csp: string[] }).__csp ??= [];
        (window as unknown as { __csp: string[] }).__csp.push(`${e.violatedDirective} ${e.blockedURI}`);
      });
    });
    page.on("console", (m) => {
      if (/content security policy/i.test(m.text())) violations.push(m.text());
    });
    await page.goto(`/${locale}/signin`);
    await page.waitForLoadState("networkidle");
    const recorded = await page.evaluate(() => (window as unknown as { __csp?: string[] }).__csp ?? []);
    expect([...violations, ...recorded]).toEqual([]);
  });
}

test("no CSP violation on the Lily pages (calendar, agenda) when signed in", async ({ page }) => {
  const seen: string[] = [];
  page.on("console", (m) => {
    if (/content security policy/i.test(m.text())) seen.push(m.text());
  });
  await page.addInitScript(() => {
    document.addEventListener("securitypolicyviolation", (e) => {
      console.error(`Content Security Policy: ${e.violatedDirective} ${e.blockedURI}`);
    });
  });
  await keycloakLogin(page, "patient");
  await expect(page.getByRole("heading", { name: "My appointments" })).toBeVisible();
  await page.waitForLoadState("networkidle");
  expect(seen).toEqual([]);
});

// NFR-6, NFR-30: at 390x844 every route passes axe in the light and dark themes and does not scroll sideways.
// Run by the `mobile` project.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { AXE_TAGS, LINK_ROUTES, PATIENT_ROUTES, PUBLIC_ROUTES, STAFF_AND_ADMIN_ROUTES } from "./support/a11y-routes";
import { keycloakLogin } from "./support/login";

async function audit(page: Page, path: string) {
  await page.goto(`/en-001${path}`);
  await page.waitForLoadState("networkidle");
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, `${path} scrolls sideways by ${overflow}px`).toBeLessThanOrEqual(1);
  const results = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
  const bad = results.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(
    bad.map(
      (v) =>
        `${v.id} (${v.impact}): ${v.nodes
          .slice(0, 2)
          .map((n) => n.target.join(" "))
          .join(", ")}`,
    ),
    path,
  ).toEqual([]);
}

for (const scheme of ["light", "dark"] as const) {
  test.describe(scheme, () => {
    // A complete authenticated route sweep runs several axe analyses in one browser context.
    test.describe.configure({ timeout: 90_000 });
    test.use({ colorScheme: scheme });

    test("public and link pages pass axe and fit", async ({ page }) => {
      for (const path of [...PUBLIC_ROUTES, ...LINK_ROUTES]) await audit(page, path);
    });

    test("patient pages pass axe and fit", async ({ page }) => {
      await keycloakLogin(page, "patient");
      for (const path of PATIENT_ROUTES) await audit(page, path);
    });

    test("staff and administrator pages pass axe and fit", async ({ page }) => {
      await keycloakLogin(page, "admin");
      for (const path of STAFF_AND_ADMIN_ROUTES) await audit(page, path);
    });
  });
}

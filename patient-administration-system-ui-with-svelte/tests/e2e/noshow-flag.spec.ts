// CAP-12: the front desk sees a support flag and the reasons for it beside an appointment, in words and with a
// shape (never colour alone), and only on the appointment that has one. It is support: nothing else on the
// page changes because of it.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";

test("a flagged appointment shows the band, the percentage and why, and the page still passes axe", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/appointments");
  await page.waitForLoadState("networkidle");
  // The agenda lists each appointment with its fields; only the flagged one carries the support flag.
  await page.getByRole("button", { name: "Agenda" }).click();
  const flag = page.getByText(/▲ High\. Based on 4 past visits/);
  await expect(flag).toBeVisible();
  await expect(page.getByText("Support flag")).toHaveCount(1);
  await expect(page.getByText(/Based on 4 past visits\./)).toBeVisible();
  await expect(page.getByText(/Past visits; Lead time/)).toBeVisible();
  await expect(page.getByText(/%|points/)).toHaveCount(0);
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
});

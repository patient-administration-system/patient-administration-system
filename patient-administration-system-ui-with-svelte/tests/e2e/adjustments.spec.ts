// NEW-6: the front desk sees a patient's reasonable adjustments beside the appointment, in words, and only on
// the appointment whose patient has any. It is information for staff: nothing else on the page changes.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";

test("an appointment shows its patient's adjustments in words, and the page still passes axe", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/appointments");
  await page.waitForLoadState("networkidle");
  await page.getByRole("button", { name: "Agenda" }).click();
  await expect(page.getByText("Large print; Step-free access")).toBeVisible();
  await expect(page.getByText("Reasonable adjustments")).toHaveCount(1);
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual(
    [],
  );
});

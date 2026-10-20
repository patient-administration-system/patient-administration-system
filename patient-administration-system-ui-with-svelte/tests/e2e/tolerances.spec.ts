// Tolerances: each measure in red, amber or green, with a mark and a word as well as a colour.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

const axe = (page: import("@playwright/test").Page) =>
  new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
const serious = (r: Awaited<ReturnType<typeof axe>>) => r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id);

test("each measure shows its tier as a word, not only a colour, and the worst one is the overall", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/tolerances");
  await expect(page.getByRole("heading", { name: "Tolerances: red, amber, green" })).toBeVisible();
  await expect(page.getByTestId("overall")).toContainText("Red");

  const row = (metric: string) => page.locator(`tr[data-metric="${metric}"]`);
  await expect(row("wait_p90_weeks")).toContainText("Amber");
  await expect(row("wait_p90_weeks")).toContainText("19 weeks");
  await expect(row("lead_time_median_days")).toContainText("Green");
  await expect(row("did_not_attend_rate")).toContainText("Red");
  await expect(row("did_not_attend_rate")).toContainText("19%");
  await expect(page.getByRole("columnheader", { name: "Green within" }).first()).toBeVisible();
  await expect(page.getByRole("columnheader", { name: "Red beyond" }).first()).toBeVisible();
  // Target, the drift that is still green, and the drift past which it is red.
  await expect(row("did_not_attend_rate").getByRole("cell").nth(1)).toHaveText("19%");
  await expect(row("did_not_attend_rate").getByRole("cell").nth(2)).toHaveText("8%");
  await expect(row("did_not_attend_rate").getByRole("cell").nth(3)).toHaveText("2%");
  await expect(row("did_not_attend_rate").getByRole("cell").nth(4)).toHaveText("6%");
  // No data is its own tier and is never green.
  await expect(row("bed_occupancy")).toContainText("No data");
  await expect(row("bed_occupancy")).not.toContainText("Green");
  await expect(row("invoice_collection_rate")).toContainText("Money");
  expect(serious(await axe(page))).toEqual([]);
});

test("the period can be changed, and only known periods are used", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/tolerances?days=7");
  await expect(page.getByText("Last 7 days")).toHaveAttribute("aria-current", "page");
  await page.goto("/en-001/tolerances?days=9999");
  await expect(page.getByText("Last 30 days")).toHaveAttribute("aria-current", "page");
});

test("the page passes axe in Welsh and Arabic too, and a patient cannot open it", async ({ page }) => {
  await keycloakLogin(page, "admin");
  for (const l of ["cy-001", "ar-001"]) {
    await page.goto(`/${l}/tolerances`);
    await expect(page.locator("tr[data-metric]")).toHaveCount(6);
    expect(serious(await axe(page))).toEqual([]);
  }
});

test("a patient does not see the tolerances", async ({ page }) => {
  await keycloakLogin(page, "patient");
  await page.goto("/en-001/tolerances");
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Tolerances: red, amber, green" })).toHaveCount(0);
});

test("each service and ward is shown against its own tolerance", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/tolerances");
  const svc = page.locator('table[data-scoped="service"]');
  await expect(svc.getByRole("rowheader", { name: "cardiology" }).first()).toBeVisible();
  const dna = svc.locator('tr[data-scoped-metric="did_not_attend_rate"]');
  await expect(dna).toContainText("Red");
  await expect(dna).toContainText("21%");
  await expect(svc.locator('tr[data-scoped-metric="slot_utilisation"]')).toContainText("Green");
  const ward = page.locator('table[data-scoped="ward"]');
  await expect(ward.locator('tr[data-scope-key="W7"]')).toContainText("Green");
  // The whole-service table still has exactly the six measures.
  await expect(page.locator("tr[data-metric]")).toHaveCount(6);
  expect(serious(await axe(page))).toEqual([]);
});

test("an open alert can be acknowledged, as the signed-in person", async ({ page, request }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/tolerances");
  const alert = page.locator('tr[data-alert="aaaaaaaa-0000-4000-8000-00000000000a"]');
  await expect(alert).toContainText("cardiology");
  await expect(alert).toContainText("21%");
  await alert.getByRole("button", { name: "Acknowledge" }).click();
  await expect(page.getByRole("status")).toHaveText("Alert acknowledged.");
  await expect(alert).toContainText("Acknowledged by");
  await expect(alert.getByRole("button")).toHaveCount(0);
  const acts = (await (await request.get(`${MOCK_URL}/__acts`)).json()) as { token: string; action: string }[];
  expect(acts.some((a) => a.action === "tolerance-alert-acknowledged" && a.token === "aaaaaaaa-0000-4000-8000-00000000000a")).toBe(true);
});

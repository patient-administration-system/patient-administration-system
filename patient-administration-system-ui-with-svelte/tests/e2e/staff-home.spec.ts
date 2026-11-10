// WPAS-5 (FR-219): the staff home page shows a tile for each area the role may use, with the suspected
// conditions past their target counted, and patients cannot reach it. WPAS-1, WPAS-3 and WPAS-4: the
// suspected-conditions list, patient enquiry and the ward overview, each passing axe.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";

const axe = (page: import("@playwright/test").Page) =>
  new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
const serious = (r: Awaited<ReturnType<typeof axe>>) =>
  r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id);

test("staff land on a home page of tiles for their own areas, with the count past target", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/");
  await expect(page).toHaveURL(/\/en-001\/home$/);
  const tiles = page.getByRole("list", { name: "Areas of work" });
  for (const name of ["Appointments", "Waitlist", "Suspected conditions", "Ward overview", "Reminder templates"]) {
    await expect(tiles.getByRole("link", { name })).toBeVisible();
  }
  await expect(tiles.getByRole("link", { name: "Home" })).toHaveCount(0);
  await expect(tiles.getByText("1 past target")).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
});

test("a patient cannot open the staff home page", async ({ page }) => {
  await keycloakLogin(page, "patient");
  const res = await page.goto("/en-001/home");
  expect(res?.status()).toBe(403);
});

test("suspected conditions are listed longest wait first, and filter by condition", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/suspicions");
  const table = page.getByRole("table", { name: "Open suspected conditions, longest wait first" });
  await expect(table).toBeVisible();
  await expect(table.getByText("Lung")).toBeVisible();
  await expect(table.getByText("Past 62 days")).toBeVisible();
  await expect(table.getByText("No target set")).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
  await page.getByLabel("Condition").selectOption("obstructive_sleep_apnoea");
  await page.getByRole("button", { name: "Show" }).click();
  await expect(table.getByText("Lung")).toHaveCount(0);
  await expect(table.getByText("Obstructive sleep apnoea")).toBeVisible();
});

test("patient enquiry and the ward overview show what they hold, and a bad id is said plainly", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/enquiry?patient=00000000-0000-0000-0000-000000000000");
  await expect(page.getByRole("heading", { name: "Suspected conditions" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Letters and messages" })).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
  await page.goto("/en-001/enquiry?patient=not-an-id");
  await expect(page.getByRole("alert")).toHaveText("That is not a valid id.");

  await page.goto("/en-001/wards?ward=77777777-7777-7777-7777-777777777777");
  const beds = page.getByRole("table", { name: "Beds by room" });
  await expect(beds).toBeVisible();
  await expect(page.getByText("1 of 2 beds occupied")).toBeVisible();
  await expect(beds.getByText("Empty")).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
});

test("a patient's name in the search opens their enquiry", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/patients?q=ada");
  await page.getByRole("link", { name: "Ada Lovelace" }).click();
  await expect(page).toHaveURL(/\/en-001\/enquiry\?patient=[0-9a-f-]+$/);
  await expect(page.getByRole("heading", { name: "Suspected conditions" })).toBeVisible();
});

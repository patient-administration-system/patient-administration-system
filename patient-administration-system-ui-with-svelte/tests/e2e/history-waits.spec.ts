// CAP-6 and CAP-8: the staff views of an appointment's history and of long waiters, read only except for
// recording that someone asked a patient. Both pass axe, and patients cannot reach them.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

const axe = (page: import("@playwright/test").Page) =>
  new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
const serious = (r: Awaited<ReturnType<typeof axe>>) => r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id);

test("the history of an appointment is a table in order, and the page passes axe", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/history?appointment=44444444-4444-4444-4444-444444444444");
  await expect(page.getByRole("table", { name: /oldest first/ })).toBeVisible();
  const rows = page.getByRole("row");
  await expect(rows).toHaveCount(4); // header and three events
  await expect(page.getByText("reminder_ladder:t_minus_7")).toBeVisible();
  await expect(page.getByText("patient_confirmed")).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
});

test("a patient's contact log, and a bad id is said plainly", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/history?patient=00000000-0000-0000-0000-000000000000");
  await expect(page.getByRole("table", { name: /newest first/ })).toBeVisible();
  await page.goto("/en-001/history?appointment=not-an-id");
  await expect(page.getByRole("alert")).toHaveText("That is not a valid id.");
});

test("long waiters are listed and asking records that someone asked, as the signed-in person", async ({ page, request }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/waits");
  await expect(page.getByRole("table", { name: "Long waiters, longest first" })).toBeVisible();
  await expect(page.getByText("orthopaedics")).toBeVisible();
  await expect(page.getByText("Never")).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
  await page.getByRole("button", { name: "Ask if still needed" }).click();
  await expect(page.getByRole("status")).toHaveText("Recorded that you asked.");
  const acts = (await (await request.get(`${MOCK_URL}/__acts`)).json()) as { token: string; action: string }[];
  expect(acts.some((a) => a.action === "still-needed-asked" && a.token === "88888888-8888-8888-8888-888888888888")).toBe(true);
});

test("a patient cannot open either view", async ({ page }) => {
  await keycloakLogin(page, "patient");
  for (const path of ["/en-001/history", "/en-001/waits"]) {
    const res = await page.goto(path);
    expect([401, 403, 302, 200].includes(res?.status() ?? 0)).toBe(true);
    await expect(page.getByRole("table")).toHaveCount(0);
    await expect(page.getByRole("heading", { name: /Appointment and message history|Long waits/ })).toHaveCount(0);
  }
});

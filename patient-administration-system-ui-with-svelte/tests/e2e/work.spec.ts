// The staff work queue (TSC-1): reminders before an item is due, one list, take, give back, take the next.
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { keycloakLogin } from "./support/login";

const axe = (page: import("@playwright/test").Page) =>
  new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
const serious = (r: Awaited<ReturnType<typeof axe>>) =>
  r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id);

test.describe.configure({ mode: "serial" });

test("the work page reminds a person of what is theirs and past due, and lists it as theirs", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/work");
  await expect(page.getByRole("heading", { name: "Work queue", level: 1 })).toBeVisible();
  await expect(page.getByTestId("reminders")).toContainText("1 overdue");
  // "Mine" is the default view: the overdue call, marked overdue, owned by "You".
  const call = page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000001"]');
  await expect(call).toContainText("Ring the patient about their appointment");
  await expect(call).toContainText("Overdue");
  await expect(call).toContainText("You");
  await expect(call.getByRole("button", { name: "Give it back" })).toBeVisible();
  expect(serious(await axe(page))).toEqual([]);
});

test("an item nobody has can be taken, then given back", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/work?view=unassigned&queue=booking");
  const follow = page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000002"]');
  await expect(follow).toContainText("Nobody");
  await follow.getByRole("button", { name: "Take it" }).click();
  await expect(page.getByRole("status").filter({ hasText: "It is yours." })).toBeVisible();
  await page.goto("/en-001/work?view=mine");
  const mine = page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000002"]');
  await expect(mine).toContainText("You");
  await mine.getByRole("button", { name: "Give it back" }).click();
  await expect(page.getByRole("status").filter({ hasText: "It is back in its queue." })).toBeVisible();
});

test("taking the next item of a queue takes the most urgent open one", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/work?view=all&queue=operations");
  await page.getByLabel("Take from the queue").selectOption("operations");
  await page.getByRole("button", { name: "Take the next item" }).click();
  await expect(page.getByRole("status").filter({ hasText: "The next item is yours." })).toBeVisible();
  await page.goto("/en-001/work?view=mine&queue=operations");
  await expect(page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000003"]')).toContainText("You");
});

test("the work page is in the staff navigation", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/home");
  await expect(
    page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Work", exact: true }),
  ).toBeVisible();
});

// Saved views (TSC-7).
test("a view someone else shared can be used but not changed", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/work");
  const shared = page.locator('li[data-view="ffffffff-0000-4000-8000-000000000001"]');
  // Names are wrapped in Unicode isolates (FSI and PDI) so a right-to-left name cannot reorder the sentence.
  await expect(shared).toContainText(/made by .?Team lead.?\)/);
  await expect(shared).toContainText(/shared with .?booking.?\)/);
  await shared.getByRole("link", { name: "Booking team: overdue" }).click();
  await expect(page.locator("table[data-work] caption")).toHaveText("Booking team: overdue");
  await expect(page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000001"]')).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete this view" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Share", exact: true })).toHaveCount(0);
  expect(serious(await axe(page))).toEqual([]);
});

test("a person saves what is shown, shares it, stops sharing it and deletes it", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/work?view=unassigned&queue=booking");
  await page.getByLabel("Save what is shown as").fill("Booking, nobody's");
  await page.getByRole("button", { name: "Save view" }).click();
  await expect(page.getByRole("status").filter({ hasText: "The view is saved." })).toBeVisible();
  await page.getByRole("link", { name: "Booking, nobody's" }).click();
  await expect(page.locator("table[data-work] caption")).toHaveText("Booking, nobody's");
  await expect(page.locator('tr[data-item="eeeeeeee-0000-4000-8000-000000000002"]')).toContainText("Nobody");
  // The same name again is refused.
  await page.goto("/en-001/work?view=unassigned&queue=booking");
  await page.getByLabel("Save what is shown as").fill("Booking, nobody's");
  await page.getByRole("button", { name: "Save view" }).click();
  await expect(page.getByRole("alert")).toContainText("already one of your views");

  await page.getByRole("link", { name: "Booking, nobody's" }).click();
  await page.getByLabel("Share with the role").fill("booking-team");
  await page.getByRole("button", { name: "Share", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "The view is shared." })).toBeVisible();
  await expect(page.getByTestId("saved-views")).toContainText(/shared with .?booking-team.?\)/);
  await page.getByRole("button", { name: "Stop sharing" }).click();
  await expect(page.getByRole("status").filter({ hasText: "The view is no longer shared." })).toBeVisible();
  await page.getByRole("button", { name: "Delete this view" }).click();
  await expect(page.getByRole("status").filter({ hasText: "The view is deleted." })).toBeVisible();
  await page.goto("/en-001/work");
  await expect(page.getByRole("link", { name: "Booking, nobody's" })).toHaveCount(0);
});

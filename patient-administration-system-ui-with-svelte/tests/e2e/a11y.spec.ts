// NFR-6: WCAG 2.2 AA, checked automatically with axe on every route.
// This is only the automated part. Automated checks find roughly a third of accessibility
// problems; a keyboard and screen-reader pass, and a published statement, are still owed (DTAC-12).
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { keycloakLogin } from "./support/login";

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

/** Page routes, from the route tree: `src/routes/my/messages/+page.svelte` is `/my/messages`. */
function routes(dir = join(process.cwd(), "src/routes"), prefix = ""): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return routes(p, `${prefix}/${name}`);
    return name === "+page.svelte" ? [prefix || "/"] : [];
  });
}

// Every route is audited somewhere below. Add a new route to one of these lists, or the coverage
// test fails. `/` only redirects, so it is audited through where it lands.
const PUBLIC = ["/signin"];
// The reply-link page needs a token in its address, so it is audited by its own tests below.
const LINKS = ["/r/[token]", "/o/[token]"];
const PATIENT = ["/my/appointments", "/my/messages"];
// `/patients` shows its data grid only after a search, so audit it with one.
const STAFF_AND_ADMIN = ["/appointments", "/waitlist", "/schedule", "/patients?q=ada", "/admin/reminders"];

async function audit(page: Page, path: string, locale = "en-001") {
  await page.goto(`/${locale}${path}`);
  await page.waitForLoadState("networkidle");
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  const bad = results.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(
    bad.map((v) => `${v.id} (${v.impact}): ${v.nodes.slice(0, 2).map((n) => n.target.join(" ")).join(", ")}`),
    `${locale}${path}`,
  ).toEqual([]);
}

test("every route is audited (a new route fails this until it is added above)", () => {
  const known = new Set(["/", ...PUBLIC, ...LINKS, ...PATIENT, ...STAFF_AND_ADMIN].map((r) => r.split("?")[0]));
  expect(routes().filter((r) => !known.has(r))).toEqual([]);
});

for (const locale of ["en-001", "cy-001", "ar-001", "zh-cn"]) {
  test(`sign-in page, ${locale}`, async ({ page }) => {
    await audit(page, "/signin", locale);
  });
}

const LINK_TOKEN = "a".repeat(64);
for (const locale of ["en-001", "cy-001", "ar-001"]) {
  test(`reply-link page, ${locale}`, async ({ page }) => {
    await audit(page, `/r/${LINK_TOKEN}`, locale);
  });
}

for (const locale of ["en-001", "cy-001", "ar-001"]) {
  test(`offer-link page, ${locale}`, async ({ page }) => {
    await audit(page, `/o/${"a".repeat(64)}`, locale);
  });
}

test("reply-link page, the are-you-sure step", async ({ page }) => {
  await page.goto(`/en-001/r/${"b".repeat(64)}`);
  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await expect(page.getByRole("heading", { name: "Are you sure?" })).toBeVisible();
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
});

for (const locale of ["en-001", "cy-001", "ar-001"]) {
  test(`reply-link page, the choose-a-new-time step, ${locale}`, async ({ page }) => {
    await page.goto(`/${locale}/r/${"a".repeat(64)}`);
    await page.locator('form[action="?/reschedule"] button').click();
    await expect(page.locator('form[action="?/rebook"] button').first()).toBeVisible();
    const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
    expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
  });
}

test("patient pages", async ({ page }) => {
  await keycloakLogin(page, "patient");
  for (const r of PATIENT) await audit(page, r);
});

test("staff and administrator pages", async ({ page }) => {
  await keycloakLogin(page, "admin");
  for (const r of STAFF_AND_ADMIN) await audit(page, r);
});

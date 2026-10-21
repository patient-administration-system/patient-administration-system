// CAP-13: the page behind a link in a reminder message. It is public (the patient is not signed in), and
// the properties that matter are about what a *visit* does:
//  - opening it, however often, changes nothing (scanners and previews follow links);
//  - only a deliberate tap acts, and Cancel asks twice;
//  - a link works once;
//  - it is readable in any language and leaks nothing onward.
import { randomBytes } from "node:crypto";
import { expect, test } from "@playwright/test";
import { MOCK_URL } from "./support/ports";

const token = () => randomBytes(32).toString("hex");
const acts = async (request: import("@playwright/test").APIRequestContext, t: string) =>
  ((await (await request.get(`${MOCK_URL}/__acts`)).json()) as { token: string; action: string }[])
    .filter((a) => a.token === t)
    .map((a) => a.action);

test("opening a link shows the appointment and four choices, and changes nothing", async ({ page, request }) => {
  const t = token();
  for (let i = 0; i < 3; i++) await page.goto(`/en-001/r/${t}`); // a scanner, a preview, a curious thumb
  await expect(page.getByRole("heading", { name: "Your appointment" })).toBeVisible();
  await expect(page.getByText(/Your appointment is on .*2026/)).toBeVisible();
  for (const name of ["Confirm that I am coming", "I need a different time", "Cancel my appointment", "I need help"]) {
    await expect(page.getByRole("button", { name })).toBeVisible();
  }
  expect(await acts(request, t), "a GET must never act").toEqual([]);
});

test("confirming works once and then the link is used up", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "Confirm that I am coming" }).click();
  await expect(page.getByText("Your appointment is confirmed.")).toBeVisible();
  expect(await acts(request, t)).toEqual(["confirm"]);
  await page.goto(`/en-001/r/${t}`);
  await expect(page.getByText(/already been used/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel my appointment" })).toHaveCount(0);
});

test("cancelling asks are-you-sure first; no keeps the appointment, yes cancels it", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await expect(page.getByRole("heading", { name: "Are you sure?" })).toBeVisible();
  expect(await acts(request, t), "the first tap changed nothing").toEqual([]);

  await page.getByRole("link", { name: "No, keep my appointment" }).click();
  await expect(page.getByRole("button", { name: "Confirm that I am coming" })).toBeVisible();
  expect(await acts(request, t)).toEqual([]);

  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await page.getByRole("button", { name: "Yes, cancel my appointment" }).click();
  await expect(page.getByText(/Your appointment is cancelled/)).toBeVisible();
  expect(await acts(request, t)).toEqual(["cancel"]);
});

test("a cancelling patient may say why, is answered with help that fits, and can skip it", async ({
  page,
  request,
}) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await expect(page.getByText("Can you tell us why?")).toBeVisible();
  await page.getByLabel("I cannot get there").check();
  await page.getByRole("button", { name: "Yes, cancel my appointment" }).click();
  await expect(page.getByText(/Your appointment is cancelled/)).toBeVisible();
  await expect(page.getByText("Help for cannot_travel. Cancelling will not affect your care.")).toBeVisible();
  expect(await acts(request, t)).toEqual(["cancel:cannot_travel"]);

  // Skipping the question still cancels, with no reason sent.
  const t2 = token();
  await page.goto(`/en-001/r/${t2}`);
  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await page.getByRole("button", { name: "Yes, cancel my appointment" }).click();
  await expect(page.getByText(/Your appointment is cancelled/)).toBeVisible();
  expect(await acts(request, t2)).toEqual(["cancel"]);
});

test("reschedule asks for a person, and help does not use the link up", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "I need help" }).click();
  await expect(page.getByText(/call the number on your appointment letter/)).toBeVisible();
  expect(await acts(request, t)).toEqual([]);
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "I need a different time" }).click();
  await expect(page.getByText(/We will contact you to find a new time/)).toBeVisible();
  expect(await acts(request, t)).toEqual(["reschedule"]);
});

test("an invalid, malformed or expired link says so plainly", async ({ page }) => {
  await page.goto(`/en-001/r/${"0".repeat(64)}`);
  await expect(page.getByText(/This link is not valid/)).toBeVisible();
  await page.goto("/en-001/r/not-a-token");
  await expect(page.getByText(/This link is not valid/)).toBeVisible();
  await page.goto(`/en-001/r/${"e".repeat(64)}`);
  await expect(page.getByText(/This link has expired/)).toBeVisible();
});

const offeredToken = (prefix: string) => prefix + randomBytes(30).toString("hex").slice(0, 60);

test("reschedule offers real times; looking does not use the link; choosing one books it once", async ({
  page,
  request,
}) => {
  const t = offeredToken("aaaa");
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "I need a different time" }).click();
  await expect(page.getByRole("heading", { name: "Choose a new time" })).toBeVisible();
  // Two times, formatted in the page's language and the clinic's zone (14:00 UTC in November is 2 pm in London).
  await expect(page.getByRole("button", { name: /10 November 2026 at 2:00 pm/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /11 November 2026 at 9:00 am/ })).toBeVisible();
  expect(await acts(request, t), "looking at times is not acting").toEqual(["reschedule-offered"]);

  // Going back leaves everything as it was.
  await page.getByRole("link", { name: "Keep my appointment as it is" }).click();
  await expect(page.getByRole("button", { name: "Confirm that I am coming" })).toBeVisible();

  await page.getByRole("button", { name: "I need a different time" }).click();
  await page.getByRole("button", { name: /10 November 2026 at 2:00 pm/ }).click();
  await expect(page.getByText(/Your appointment has moved to .*10 November 2026/)).toBeVisible();
  expect((await acts(request, t)).filter((a) => a.startsWith("rebook"))).toEqual([
    "rebook:11111111-1111-4111-8111-111111111111",
  ]);
  await page.goto(`/en-001/r/${t}`);
  await expect(page.getByText(/already been used/)).toBeVisible();
});

test("if the chosen time has just gone, the patient is told and can ask again", async ({ page }) => {
  const t = offeredToken("bbbb");
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "I need a different time" }).click();
  await page.getByRole("button", { name: /10 November 2026 at 2:00 pm/ }).click();
  await expect(page.getByText(/that time has just been taken/)).toBeVisible();
  await expect(page.getByRole("button", { name: "I need a different time" })).toBeVisible();
});

test("on an urgent pathway a cancel is held: the patient is told someone will phone first", async ({
  page,
  request,
}) => {
  const t = "dddd" + randomBytes(30).toString("hex").slice(0, 60);
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "Cancel my appointment" }).click();
  await page.getByRole("button", { name: "Yes, cancel my appointment" }).click();
  await expect(page.getByText(/has not been cancelled.*someone will phone you first/)).toBeVisible();
  expect(await acts(request, t)).toEqual(["cancel-held"]);
});

test("the page reads in Welsh and Arabic, the latter right to left", async ({ page }) => {
  const t = token();
  await page.goto(`/cy-001/r/${t}`);
  await expect(page.getByRole("heading", { name: "Eich apwyntiad" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Canslo fy apwyntiad" })).toBeVisible();
  await page.goto(`/ar-001/r/${t}`);
  await expect(page.getByRole("heading", { name: "موعدك" })).toBeVisible();
  expect(await page.locator("html").getAttribute("dir")).toBe("rtl");
});

test("the page is not cached, does not leak the address onward, and is not indexed", async ({ request, page }) => {
  const t = token();
  const res = await request.get(`/en-001/r/${t}`);
  expect(res.headers()["cache-control"]).toContain("no-store");
  expect(res.headers()["referrer-policy"]).toBe("same-origin");
  await page.goto(`/en-001/r/${t}`);
  expect(await page.locator('meta[name="robots"]').getAttribute("content")).toBe("noindex");
});

test("the buttons work with scripts off, as a slow phone or a locked-down browser would have them", async ({
  browser,
  baseURL,
  request,
}) => {
  // A plain form post carries `Origin: null` under `Referrer-Policy: no-referrer`, which SvelteKit refuses as
  // cross-site: the page's policy is `same-origin` so this works.
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
  const page = await context.newPage();
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await page.getByRole("button", { name: "Confirm that I am coming" }).click();
  await expect(page.getByText("Your appointment is confirmed.")).toBeVisible();
  expect(await acts(request, t)).toEqual(["confirm"]);
  await context.close();
});

test("a bare link gains a language prefix and still works", async ({ page }) => {
  const t = token();
  await page.goto(`/r/${t}`);
  await expect(page).toHaveURL(new RegExp(`/[a-z]{2,3}-[a-z0-9]{2,3}/r/${t}$`));
  await expect(page.getByRole("button", { name: /.+/ }).first()).toBeVisible();
});

// PILOT-8: the page behind a link in an "an appointment has become free" message. Public, and the properties
// that matter are about what a *visit* does: opening it changes nothing, only a tap accepts, it works once.
import { randomBytes } from "node:crypto";
import { expect, test } from "@playwright/test";
import { MOCK_URL } from "./support/ports";

const token = (prefix = "") => prefix + randomBytes(32).toString("hex").slice(prefix.length);
const acts = async (request: import("@playwright/test").APIRequestContext, t: string) =>
  ((await (await request.get(`${MOCK_URL}/__acts`)).json()) as { token: string; action: string }[])
    .filter((a) => a.token === t)
    .map((a) => a.action);

test("opening an offer shows the time and two choices, and changes nothing", async ({ page, request }) => {
  const t = token();
  for (let i = 0; i < 3; i++) await page.goto(`/en-001/o/${t}`); // a scanner, a preview, a curious thumb
  await expect(page.getByRole("heading", { name: "An appointment has become free" })).toBeVisible();
  await expect(page.getByText(/An appointment is free on .*2026.*Would you like it\?/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Yes, I will take it" })).toBeVisible();
  await expect(page.getByRole("button", { name: "No, thank you" })).toBeVisible();
  expect(await acts(request, t), "a GET must never take an appointment").toEqual([]);
});

test("accepting books it once and the link is then used up", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/o/${t}`);
  await page.getByRole("button", { name: "Yes, I will take it" }).click();
  await expect(page.getByText(/Your appointment is on .*5 November 2026/)).toBeVisible();
  expect(await acts(request, t)).toEqual(["offer-accept"]);
  await page.goto(`/en-001/o/${t}`);
  await expect(page.getByText(/already been used/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Yes, I will take it" })).toHaveCount(0);
});

test("declining says thank you", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/o/${t}`);
  await page.getByRole("button", { name: "No, thank you" }).click();
  await expect(page.getByText("Thank you for letting us know.")).toBeVisible();
  expect(await acts(request, t)).toEqual(["offer-decline"]);
});

test("a slot taken meanwhile is said plainly, and an expired offer is expired", async ({ page }) => {
  const g = token("9999");
  await page.goto(`/en-001/o/${g}`);
  await page.getByRole("button", { name: "Yes, I will take it" }).click();
  await expect(page.getByText("Sorry, this appointment has just been taken.")).toBeVisible();
  const e = token("eeee");
  await page.goto(`/en-001/o/${e}`);
  await expect(page.getByText(/has expired/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Yes, I will take it" })).toHaveCount(0);
});

test("it reads in Welsh and Arabic, is not cached, leaks nothing onward and is not indexed", async ({ page, request }) => {
  const t = token();
  await page.goto(`/cy-001/o/${t}`);
  await expect(page.getByRole("heading", { name: "Mae apwyntiad ar gael" })).toBeVisible();
  await page.goto(`/ar-001/o/${t}`);
  expect(await page.locator("html").getAttribute("dir")).toBe("rtl");
  const res = await request.get(`/en-001/o/${t}`);
  expect(res.headers()["cache-control"]).toContain("no-store");
  expect(res.headers()["referrer-policy"]).toBe("no-referrer");
  await page.goto(`/en-001/o/${t}`);
  expect(await page.locator('meta[name="robots"]').getAttribute("content")).toBe("noindex");
});

test("a bad address is just not valid", async ({ page }) => {
  await page.goto(`/en-001/o/not-a-token`);
  await expect(page.getByText(/not valid/)).toBeVisible();
});

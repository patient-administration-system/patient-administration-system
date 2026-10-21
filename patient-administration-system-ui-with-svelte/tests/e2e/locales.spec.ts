// NFR-4 / NFR-5 in a browser: the URL decides the language, the document says so, and Arabic
// reads right to left.
import { expect, test } from "@playwright/test";

test("a bare URL gains the language the browser asks for", async ({ browser }) => {
  const context = await browser.newContext({ locale: "cy-GB", extraHTTPHeaders: { "accept-language": "cy" } });
  const page = await context.newPage();
  await page.goto("/signin");
  await expect(page).toHaveURL(/\/cy-001\/signin/);
  await expect(page.locator("html")).toHaveAttribute("lang", "cy-001");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Mewngofnodi");
  await context.close();
});

test("English is the default for a language we do not have", async ({ browser }) => {
  const context = await browser.newContext({ extraHTTPHeaders: { "accept-language": "xx-YY" } });
  const page = await context.newPage();
  await page.goto("/signin");
  await expect(page).toHaveURL(/\/en-001\/signin/);
  await context.close();
});

test("a two-letter prefix is not a locale", async ({ page }) => {
  const res = await page.goto("/en/signin");
  // `/en/signin` is an ordinary unprefixed path: it gains a locale and then finds no page.
  await expect(page).toHaveURL(/\/en-001\/en\/signin/);
  expect(res?.status()).toBe(404);
});

test("Arabic is right to left; the others are left to right", async ({ page }) => {
  await page.goto("/ar-001/signin");
  await expect(page.locator("html")).toHaveAttribute("dir", "rtl");
  await expect(page.locator("html")).toHaveAttribute("lang", "ar-001");
  await page.goto("/de-001/signin");
  await expect(page.locator("html")).toHaveAttribute("dir", "ltr");
});

test("the language menu keeps you on the same page", async ({ page }) => {
  await page.goto("/en-001/signin?error=denied");
  await page.getByText("Language").click();
  await page.getByRole("link", { name: "Cymraeg" }).click();
  await expect(page).toHaveURL(/\/cy-001\/signin\?error=denied/);
  await expect(page.getByRole("alert")).toHaveText("Cafodd y mewngofnodi ei ganslo.");
});

test("every language shows the sign-in page in that language", async ({ page }) => {
  const seen = new Set<string>();
  for (const code of [
    "ar-001",
    "cy-001",
    "de-001",
    "de-de",
    "en-001",
    "es-001",
    "fr-001",
    "hi-001",
    "zh-001",
    "zh-cn",
  ]) {
    await page.goto(`/${code}/signin`);
    const heading = (await page.getByRole("heading", { level: 1 }).textContent()) ?? "";
    expect(heading.trim().length, code).toBeGreaterThan(0);
    seen.add(heading.trim());
  }
  // Ten locales, six distinct words for "Sign in" at least (de-001/de-de and zh-001/zh-cn are identical).
  expect(seen.size).toBeGreaterThanOrEqual(6);
});

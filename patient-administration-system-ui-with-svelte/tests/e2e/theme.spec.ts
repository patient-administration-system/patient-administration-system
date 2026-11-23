// NFR-30, UI-4: the browser's light or dark setting picks the theme until the person chooses one with the
// Lily theme picker in the header (Follow this device, Light, Dark). The picker keeps the choice in the
// `theme` cookie, which the server reads for the first paint, and in localStorage (`pas-theme`).
import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { keycloakLogin } from "./support/login";

/** The colour scheme the page is painted in, and the `data-theme` on <html>. */
async function painted(page: Page) {
  return page.evaluate(() => ({
    scheme: getComputedStyle(document.documentElement).colorScheme,
    dataTheme: document.documentElement.dataset.theme ?? "",
    background: getComputedStyle(document.body).backgroundColor,
  }));
}

async function choose(page: Page, option: string, menu = "Theme") {
  await page.getByRole("button", { name: menu }).click();
  await page.getByRole("listbox", { name: menu }).getByRole("option", { name: option, exact: true }).click();
}

async function kept(page: Page, context: BrowserContext) {
  const cookie = (await context.cookies()).find((c) => c.name === "theme")?.value;
  const stored = await page.evaluate(() => localStorage.getItem("pas-theme"));
  return { cookie, stored };
}

test.describe("with no choice made", () => {
  test("a dark browser gets the dark theme on the first paint, and nothing is kept", async ({ browser }) => {
    const context = await browser.newContext({ colorScheme: "dark" });
    const page = await context.newPage();
    const response = await page.goto("/en-001/signin");
    // The served HTML leaves the device in charge: nothing waits for a script.
    expect(await response!.text()).toContain('data-theme=""');
    expect((await painted(page)).scheme).toBe("dark");
    expect(await kept(page, context)).toEqual({ cookie: undefined, stored: null });
    await context.close();
  });

  test("a light browser gets the light theme", async ({ browser }) => {
    const context = await browser.newContext({ colorScheme: "light" });
    const page = await context.newPage();
    await page.goto("/en-001/signin");
    expect((await painted(page)).scheme).toBe("light");
    await context.close();
  });

  test("the theme follows the browser when its setting changes mid-page", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/en-001/signin");
    const light = await painted(page);
    expect(light.scheme).toBe("light");
    await page.emulateMedia({ colorScheme: "dark" });
    await expect.poll(async () => (await painted(page)).scheme).toBe("dark");
    expect((await painted(page)).background).not.toBe(light.background);
  });
});

test.describe("a person's choice", () => {
  test("Light holds against a dark browser: kept in the cookie and local storage, served on the next page", async ({
    browser,
  }) => {
    const context = await browser.newContext({ colorScheme: "dark" });
    const page = await context.newPage();
    await page.goto("/cy-001/signin");
    await choose(page, "Golau", "Thema");
    expect(await painted(page)).toMatchObject({ scheme: "light", dataTheme: "light" });
    await expect.poll(() => kept(page, context)).toEqual({ cookie: "light", stored: "light" });
    // A new tab: the server reads the cookie, so the first paint is already light.
    const tab = await context.newPage();
    const response = await tab.goto("/en-001/signin");
    expect(await response!.text()).toContain('data-theme="light"');
    expect((await painted(tab)).scheme).toBe("light");
    await tab.emulateMedia({ colorScheme: "light" });
    await tab.emulateMedia({ colorScheme: "dark" });
    expect((await painted(tab)).scheme).toBe("light");
    await context.close();
  });

  test("Dark holds against a light browser; Follow this device hands it back in one step", async ({ browser }) => {
    const context = await browser.newContext({ colorScheme: "light" });
    const page = await context.newPage();
    await page.goto("/en-001/signin");
    await choose(page, "Dark");
    expect(await painted(page)).toMatchObject({ scheme: "dark", dataTheme: "dark" });
    await choose(page, "Follow this device");
    await expect.poll(async () => (await painted(page)).scheme).toBe("light");
    await expect.poll(() => kept(page, context)).toEqual({ cookie: "system", stored: "system" });
    // The device leads again, on this page and on the next one.
    await page.emulateMedia({ colorScheme: "dark" });
    await expect.poll(async () => (await painted(page)).scheme).toBe("dark");
    await context.close();
  });

  test("the menu works from the keyboard and marks the current choice", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/en-001/signin");
    const button = page.getByRole("button", { name: "Theme" });
    await button.focus();
    await page.keyboard.press("Enter");
    const list = page.getByRole("listbox", { name: "Theme" });
    await expect(list).toBeVisible();
    await expect(list.getByRole("option", { name: "Follow this device" })).toHaveAttribute("aria-selected", "true");
    // The picker lists the device choice first, then the schemes by name: Dark comes next, Light last.
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(list).toBeHidden();
    await expect(button).toBeFocused();
    expect((await painted(page)).dataTheme).toBe("dark");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Escape");
    await expect(list).toBeHidden();
    expect((await painted(page)).dataTheme).toBe("dark");
  });

  test("the choice applies only to the browser that made it", async ({ browser }) => {
    const mine = await browser.newContext({ colorScheme: "light" });
    const page = await mine.newPage();
    await page.goto("/en-001/signin");
    await choose(page, "Dark");
    const theirs = await browser.newContext({ colorScheme: "light" });
    const other = await theirs.newPage();
    await other.goto("/en-001/signin");
    expect(await painted(other)).toMatchObject({ scheme: "light", dataTheme: "" });
    await mine.close();
    await theirs.close();
  });

  test("the choice survives signing out", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await keycloakLogin(page, "patient");
    await page.goto("/en-001/my/appointments");
    await choose(page, "Dark");
    expect((await painted(page)).scheme).toBe("dark");
    await page.getByRole("button", { name: "Sign out" }).click();
    // Through the identity provider's end-session page and back to signing in.
    await page.waitForURL(/\/en-001\/signin$/);
    expect(await painted(page)).toMatchObject({ scheme: "dark", dataTheme: "dark" });
  });

  test("an unknown cookie value follows the device", async ({ browser, baseURL }) => {
    const context = await browser.newContext({ colorScheme: "dark" });
    await context.addCookies([{ name: "theme", value: "purple", url: baseURL! }]);
    const page = await context.newPage();
    const response = await page.goto("/en-001/signin");
    expect(await response!.text()).toContain('data-theme=""');
    expect((await painted(page)).scheme).toBe("dark");
    await context.close();
  });
});

test("the privacy page says what is kept, and every page links to it", async ({ page }) => {
  await page.goto("/en-001/signin");
  await page.getByRole("contentinfo").getByRole("link", { name: "Privacy" }).click();
  await expect(page).toHaveURL(/\/en-001\/privacy$/);
  await expect(page.getByRole("heading", { level: 1, name: "Privacy" })).toBeVisible();
  for (const name of ["theme", "locale"]) await expect(page.getByText(`a cookie named ${name}`)).toBeVisible();
});

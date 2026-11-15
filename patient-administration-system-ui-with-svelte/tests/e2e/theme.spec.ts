// NFR-30, UI-4: the browser's light or dark setting picks the theme until the person chooses one in
// the theme menu (Follow this device, Light, Dark); the choice is kept in a cookie the server reads.
import { expect, test, type Page } from "@playwright/test";
import { keycloakLogin } from "./support/login";

/** The colour scheme the page is actually painted in, and the choice the server rendered. */
async function painted(page: Page) {
  return page.evaluate(() => ({
    scheme: getComputedStyle(document.documentElement).colorScheme,
    choice: document.documentElement.dataset.colorScheme,
    background: getComputedStyle(document.body).backgroundColor,
  }));
}

async function choose(page: Page, name: string) {
  await page.locator("details.themes summary").click();
  await page.locator("details.themes").getByRole("button", { name }).click();
  await page.waitForLoadState("load");
}

test.describe("with no choice made", () => {
  test("a dark browser gets the dark theme on the first paint", async ({ browser }) => {
    const context = await browser.newContext({ colorScheme: "dark" });
    const page = await context.newPage();
    // The served HTML already says to follow the device: nothing waits for a script.
    const response = await page.goto("/en-001/signin");
    expect(await response!.text()).toContain('data-color-scheme="device"');
    expect((await painted(page)).scheme).toBe("dark");
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
    const dark = await painted(page);
    expect(dark.scheme).toBe("dark");
    expect(dark.background).not.toBe(light.background);
  });
});

test.describe("a person's choice", () => {
  test("Light holds against a dark browser, in a new tab and after the browser changes", async ({ browser }) => {
    const context = await browser.newContext({ colorScheme: "dark" });
    const page = await context.newPage();
    await page.goto("/cy-001/signin?error=idle");
    await choose(page, "Golau");
    // Back on the same page, in the same language.
    expect(new URL(page.url()).pathname + new URL(page.url()).search).toBe("/cy-001/signin?error=idle");
    expect(await painted(page)).toMatchObject({ scheme: "light", choice: "light" });
    // The menu marks the current choice (it is closed after the page reloads, so look by value).
    await expect(page.locator('details.themes button[value="light"]')).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator('details.themes button[value="device"]')).toHaveAttribute("aria-pressed", "false");
    const tab = await context.newPage();
    await tab.goto("/en-001/signin");
    expect(await painted(tab)).toMatchObject({ scheme: "light", choice: "light" });
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
    expect(await painted(page)).toMatchObject({ scheme: "dark", choice: "dark" });
    await choose(page, "Follow this device");
    expect(await painted(page)).toMatchObject({ scheme: "light", choice: "device" });
    expect((await context.cookies()).find((c) => c.name === "theme")).toBeUndefined();
    await context.close();
  });

  test("the choice applies only to the browser that made it", async ({ browser }) => {
    const mine = await browser.newContext({ colorScheme: "light" });
    const page = await mine.newPage();
    await page.goto("/en-001/signin");
    await choose(page, "Dark");
    const theirs = await browser.newContext({ colorScheme: "light" });
    const other = await theirs.newPage();
    await other.goto("/en-001/signin");
    expect(await painted(other)).toMatchObject({ scheme: "light", choice: "device" });
    await mine.close();
    await theirs.close();
  });

  test("the choice survives signing out", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await keycloakLogin(page, "patient");
    await page.goto("/en-001/my/appointments");
    await choose(page, "Dark");
    expect(new URL(page.url()).pathname).toBe("/en-001/my/appointments");
    expect((await painted(page)).scheme).toBe("dark");
    await page.getByRole("button", { name: "Sign out" }).click();
    // Through the identity provider's end-session page and back to signing in.
    await page.waitForURL(/\/en-001\/signin$/);
    expect(await painted(page)).toMatchObject({ scheme: "dark", choice: "dark" });
  });
});

test.describe("the theme endpoint", () => {
  test("refuses an unknown theme and never sends anyone off the site", async ({ request, baseURL }) => {
    const headers = { origin: baseURL! };
    const unknown = await request.post("/theme", { form: { theme: "purple", back: "/" }, headers, maxRedirects: 0 });
    expect(unknown.status()).toBe(400);
    const away = await request.post("/theme", {
      form: { theme: "dark", back: "//evil.example/" },
      headers,
      maxRedirects: 0,
    });
    expect(away.status()).toBe(303);
    expect(away.headers()["location"]).toBe("/");
  });
});

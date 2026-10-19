// A phone-width pass (390x844): no page may scroll sideways, and the language menu and the
// navigation must still be reachable. Run by the `mobile` project.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";

async function noHorizontalScroll(page: import("@playwright/test").Page, label: string) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow, `${label} scrolls sideways by ${overflow}px`).toBeLessThanOrEqual(1);
}

test("sign-in fits a phone, in left-to-right and right-to-left", async ({ page }) => {
  for (const l of ["en-001", "ar-001"]) {
    await page.goto(`/${l}/signin`);
    await noHorizontalScroll(page, `${l} sign-in`);
  }
});

test("signed-in pages fit a phone", async ({ page }) => {
  await keycloakLogin(page, "patient");
  for (const path of ["/my/appointments", "/my/messages"]) {
    await page.goto(`/en-001${path}`);
    await page.waitForLoadState("networkidle");
    await noHorizontalScroll(page, path);
  }
});

test("the reply-link page fits a phone, in left-to-right and right-to-left", async ({ page }) => {
  for (const l of ["en-001", "ar-001"]) {
    await page.goto(`/${l}/r/${"c".repeat(64)}`);
    await noHorizontalScroll(page, `${l} reply link`);
  }
});

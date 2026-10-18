import type { Page } from "@playwright/test";

/** Sign in through the real Keycloak login form (seeded users: patient/patient, admin/admin). */
export async function keycloakLogin(page: Page, user: "patient" | "admin", start = "/en-001/signin") {
  await page.goto(start);
  await page.locator('a[href^="/signin/oidc"]').click();
  await page.locator("#username").fill(user);
  await page.locator("#password").fill(user);
  await page.locator("#kc-login").click();
}

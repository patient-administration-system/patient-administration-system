import type { Page } from "@playwright/test";
import { ADMIN_TOTP_SECRET, totp } from "./totp";

/**
 * Sign in through the real Keycloak login form (seeded users: patient/patient, admin/admin). The realm asks
 * staff and administrators for a one-time code as well (DTAC-5), so the administrator's is generated from the
 * seeded secret; the patient is never asked.
 */
export async function keycloakLogin(page: Page, user: "patient" | "admin", start = "/en-001/signin") {
  await page.goto(start);
  await page.locator('a[href^="/signin/oidc"]').click();
  await page.locator("#username").fill(user);
  await page.locator("#password").fill(user);
  await page.locator("#kc-login").click();
  if (user === "admin") {
    await page.locator("#otp").fill(totp(ADMIN_TOTP_SECRET));
    await page.locator("#kc-login").click();
  }
}

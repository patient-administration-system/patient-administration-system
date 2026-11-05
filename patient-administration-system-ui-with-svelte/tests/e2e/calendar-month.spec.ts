// CAL-1: the calendar names this month in the server's HTML, so it does not flash January 2000 on load and is right
// for anyone without JavaScript. `page.request` shares the signed-in cookies and runs none of the page's script.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";

const thisMonth = new Intl.DateTimeFormat("en-001", { month: "long", year: "numeric", timeZone: "UTC" }).format(
  new Date(),
);

for (const [user, path] of [
  ["admin", "/en-001/appointments"],
  ["patient", "/en-001/my/appointments"],
] as const) {
  test(`${path} names this month before any script runs`, async ({ page }) => {
    await keycloakLogin(page, user);
    const response = await page.request.get(path);
    expect(response.status()).toBe(200);
    const html = await response.text();
    expect(html).toContain(thisMonth);
    expect(html).not.toContain("January 2000");
  });
}

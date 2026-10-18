// Kanban 0.8.1 gives each card cell no aria-label of its own: as a `gridcell` it is named from its
// content, so the accessible name is always everything visible on it (title, subtitle, badges),
// which WCAG 2.5.3 (label in name) needs. 0.8.0 set an aria-label from the title alone, which hid the
// subtitle and badge from a screen reader. This pins the fixed behaviour, so a downgrade or a
// regression is caught here and not only by the axe audit.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";

test("each waitlist card is named by its title, subtitle and priority", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/en-001/waitlist");
  const urgent = page.locator("td[data-card-id='66666666-6666-6666-6666-666666666666']");
  await expect(urgent).toBeVisible();
  // The browser-computed accessible name, which is what a screen reader announces.
  await expect(urgent).toHaveAccessibleName(/general.*Patient.*0{8}.*Urgent/);

  const routine = page.locator("td[data-card-id='77777777-7777-7777-7777-777777777777']");
  await expect(routine).toHaveAccessibleName(/general.*Patient.*Routine/);

  // Every word a sighted reader sees on a card is in the name a screen reader announces.
  const visible = (await urgent.innerText()).replace(/[⁦-⁩]/g, "");
  for (const word of visible.split(/\s+/).filter((w) => /[A-Za-z0-9]/.test(w))) {
    await expect(urgent, `"${word}" is visible but missing from the accessible name`).toHaveAccessibleName(
      new RegExp(word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")),
    );
  }
});

// NFR-23: a page says when the list it shows is not the whole list, in every locale.
import { expect, test } from "@playwright/test";
import { LOCALES } from "../../src/lib/locales";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

const PATIENT = "00000000-0000-0000-0000-000000000000";

test.afterEach(async ({ request }) => {
  await request.get(`${MOCK_URL}/__cut?on=false`);
});

test("a whole list says nothing", async ({ page }) => {
  await keycloakLogin(page, "admin");
  for (const path of ["/en-001/waits", "/en-001/tolerances", `/en-001/history?patient=${PATIENT}`]) {
    await page.goto(path);
    await expect(page.getByRole("table").first()).toBeVisible();
    await expect(page.getByTestId("list-cut")).toHaveCount(0);
  }
});

test("each page with a cut list says so, and how many it shows", async ({ page, request }) => {
  await request.get(`${MOCK_URL}/__cut?on=true`);
  await keycloakLogin(page, "admin");
  const text = "This list is not complete: it shows 7, and there are more.";

  await page.goto("/en-001/waits");
  await expect(page.getByTestId("list-cut")).toHaveText(text);

  await page.goto(`/en-001/history?patient=${PATIENT}`);
  await expect(page.getByTestId("list-cut")).toHaveText(text);

  // Once for the service and ward tables, and once for the open alerts.
  await page.goto("/en-001/tolerances");
  await expect(page.getByTestId("list-cut")).toHaveCount(2);
});

test("the note is translated in every locale", async ({ page, request }) => {
  await request.get(`${MOCK_URL}/__cut?on=true`);
  await keycloakLogin(page, "admin");
  const english = "This list is not complete";
  for (const locale of LOCALES) {
    await page.goto(`/${locale}/waits`);
    const note = page.getByTestId("list-cut");
    await expect(note, locale).toContainText("7");
    if (!locale.startsWith("en")) await expect(note, locale).not.toContainText(english);
  }
});

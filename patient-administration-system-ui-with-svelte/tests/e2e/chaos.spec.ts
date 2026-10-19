// Chaos testing for the UI: the PAS API misbehaves on purpose (errors that leak internals, garbage, HTML from a
// proxy, empty bodies, the wrong JSON shape, null), and every staff page must still answer with a clear error
// page: no stack trace, no SQL, no secret, no uncaught exception in the browser, no hang. When the API recovers,
// so do the pages. A random mode picks a fault per request from a seed (`CHAOS_SEED`, default 1) so a failing run
// can be repeated.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

const STAFF_PAGES = ["/appointments", "/waitlist", "/schedule", "/patients?q=ada", "/history?appointment=44444444-4444-4444-4444-444444444444", "/waits", "/admin/reminders"];
const MODES = ["500", "garbage", "html", "empty", "wrongshape", "null"];
const LEAKS = ["hunter2", "panicked", "src/db", "SqlxError", "PgDatabaseError", "TypeError", "ReferenceError", "at async", "node_modules", "/Users/"];

async function setChaos(request: import("@playwright/test").APIRequestContext, mode: string, seed = 1) {
  await request.get(`${MOCK_URL}/__chaos?mode=${mode}&seed=${seed}`);
}

test.afterEach(async ({ request }) => {
  await setChaos(request, "off");
});

for (const mode of MODES) {
  test(`with the API answering "${mode}", every staff page fails cleanly and recovers`, async ({ page, request }) => {
    test.setTimeout(120_000);
    const uncaught: string[] = [];
    page.on("pageerror", (e) => uncaught.push(e.message));
    await keycloakLogin(page, "admin");
    for (const path of STAFF_PAGES) {
      await setChaos(request, mode);
      const res = await page.goto(`/en-001${path}`, { timeout: 20_000 });
      const html = await page.content();
      for (const leak of LEAKS) expect(html, `${path} (${mode}) leaks "${leak}"`).not.toContain(leak);
      // Either a page with its heading (the fault did not matter here) or the error page: never blank.
      await expect(page.getByRole("heading", { level: 1 }), `${path} (${mode}) has no heading`).toBeVisible();
      expect(res?.status() ?? 0, `${path} (${mode})`).toBeLessThan(600);
      await setChaos(request, "off");
    }
    // Recovered: the pages work again as soon as the API does.
    await page.goto("/en-001/waitlist");
    await expect(page.getByRole("heading", { name: "Waitlist" })).toBeVisible();
    expect(uncaught, "uncaught exceptions in the browser").toEqual([]);
  });
}

test("with the API failing at random, no page leaks, hangs or crashes (seeded)", async ({ page, request }) => {
  test.setTimeout(180_000);
  const seed = Number(process.env.CHAOS_SEED ?? 1) || 1;
  const uncaught: string[] = [];
  page.on("pageerror", (e) => uncaught.push(e.message));
  await keycloakLogin(page, "admin");
  await setChaos(request, "random", seed);
  for (let round = 0; round < 3; round++) {
    for (const path of STAFF_PAGES) {
      await page.goto(`/en-001${path}`, { timeout: 20_000 });
      const html = await page.content();
      for (const leak of LEAKS) expect(html, `seed ${seed}: ${path} leaks "${leak}"`).not.toContain(leak);
      await expect(page.getByRole("heading", { level: 1 }), `seed ${seed}: ${path} has no heading`).toBeVisible();
    }
  }
  expect(uncaught, `seed ${seed}: uncaught exceptions`).toEqual([]);
});

test("a patient's pages fail cleanly too", async ({ page, request }) => {
  await keycloakLogin(page, "patient");
  for (const mode of MODES) {
    await setChaos(request, mode);
    for (const path of ["/my/appointments", "/my/messages"]) {
      await page.goto(`/en-001${path}`, { timeout: 20_000 });
      const html = await page.content();
      for (const leak of LEAKS) expect(html, `${path} (${mode}) leaks "${leak}"`).not.toContain(leak);
      await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    }
  }
});

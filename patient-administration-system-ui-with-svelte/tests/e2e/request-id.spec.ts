// A page's calls to the PAS API carry its request id, so a failing page can be followed into the API's logs.
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

type Seen = { path: string; id: string | null };

test("the API calls a page makes carry the page's request id, which the page answers with", async ({
  page,
  request,
}) => {
  await keycloakLogin(page, "admin");
  const id = `e2e-${Date.now()}`;
  await page.setExtraHTTPHeaders({ "x-request-id": id });
  const response = await page.goto("/en-001/waits");
  expect(response?.headers()["x-request-id"]).toBe(id);
  const seen: Seen[] = await (await request.get(`${MOCK_URL}/__request-ids`)).json();
  expect(seen.filter((s) => s.id === id).map((s) => s.path)).toContain("/api/pathways/due-check");
});

test("an id of an unsafe shape is replaced, not passed on", async ({ page, request }) => {
  await keycloakLogin(page, "admin");
  await page.setExtraHTTPHeaders({ "x-request-id": "not a safe id <x>" });
  const response = await page.goto("/en-001/waits");
  const made = response?.headers()["x-request-id"] ?? "";
  expect(made).toMatch(/^[0-9a-f-]{36}$/);
  const seen: Seen[] = await (await request.get(`${MOCK_URL}/__request-ids`)).json();
  expect(seen.some((s) => s.id === "not a safe id <x>")).toBe(false);
  expect(seen.some((s) => s.id === made)).toBe(true);
});

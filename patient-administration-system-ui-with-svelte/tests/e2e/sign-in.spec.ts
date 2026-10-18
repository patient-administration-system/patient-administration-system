import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";
import { MOCK_URL } from "./support/ports";

test("signed-out visitors are sent to sign in, then back where they were going", async ({ page }) => {
  await page.goto("/my/appointments");
  // A bare URL gains the reader's locale, and the page to come back to is remembered in it.
  await expect(page).toHaveURL(/\/en-001\/signin\?return=%2Fen-001%2Fmy%2Fappointments/);
  await expect(page.getByRole("heading", { name: "Sign in" })).toBeVisible();
});

test("a patient signs in and sees only their own appointments", async ({ page, request }) => {
  await keycloakLogin(page, "patient");
  await expect(page).toHaveURL(/\/my\/appointments/);
  await expect(page.getByRole("heading", { name: "My appointments" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "Knee review" })).toBeVisible();

  // The person id sent to the PAS came from the verified ID token, not from the browser.
  const asked: string[] = await (await request.get(`${MOCK_URL}/__requests`)).json();
  expect(asked).toContain("/api/patients/00000000-0000-0000-0000-000000000000/appointments");

  // Secure portal messages: shown after sign-in, fetched for the token's own person id only.
  await page.getByRole("link", { name: "My messages" }).click();
  await expect(page.getByRole("heading", { name: "Your appointment letter" })).toBeVisible();
  const asked2: string[] = await (await request.get(`${MOCK_URL}/__requests`)).json();
  expect(asked2).toContain("/api/patients/00000000-0000-0000-0000-000000000000/letters?channel=portal");

  // A patient is not staff: the staff pages refuse them.
  const res = await page.goto("/appointments");
  expect(res?.status()).toBe(403);
  const admin = await page.goto("/admin/reminders");
  expect(admin?.status()).toBe(403);

  await page.goto("/my/appointments");
  await page.getByRole("button", { name: "Sign out" }).click();
  await page.goto("/my/appointments");
  await expect(page).toHaveURL(/\/signin/);
});

test("an administrator can open the reminder-template editor; a patient cannot", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/admin/reminders");
  await expect(page.getByRole("heading", { name: "Reminder templates" })).toBeVisible();
  await expect(page.getByLabel("Subject")).toHaveValue("Appointment reminder");
  await expect(page.getByLabel("WhatsApp")).toBeChecked();
});

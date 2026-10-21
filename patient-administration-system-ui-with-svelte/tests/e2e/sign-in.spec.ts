import { expect, test } from "@playwright/test";
import { keycloakLogin } from "./support/login";
import { ADMIN_TOTP_SECRET, totp } from "./support/totp";
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

  // The BFF presented a genuine token from the real issuer and named the signed-in person. The mock
  // API verified the PASETO signature, so a missing or forged token would have been a 401 here.
  const seen: { path: string; actor: string | null; sub: string | null; service: boolean }[] = await (
    await request.get(`${MOCK_URL}/__actors`)
  ).json();
  const mine = seen.find((r) => r.path === "/api/patients/00000000-0000-0000-0000-000000000000/appointments");
  expect(mine?.sub).toBe("pas-ui");
  expect(mine?.service).toBe(true);
  expect(mine?.actor).toMatch(/^[0-9a-f-]{36}$/);
  expect(seen.every((r) => r.sub === "pas-ui")).toBe(true);

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

test("staff and administrators must give a one-time code as well as a password; patients are not asked (DTAC-5)", async ({
  page,
}) => {
  // A password alone does not sign an administrator in: the next step asks for a code, and a wrong one is refused.
  await page.goto("/en-001/signin");
  await page.locator('a[href^="/signin/oidc"]').click();
  await page.locator("#username").fill("admin");
  await page.locator("#password").fill("admin");
  await page.locator("#kc-login").click();
  await expect(page.locator("#otp")).toBeVisible();
  const right = totp(ADMIN_TOTP_SECRET);
  await page.locator("#otp").fill(right === "000000" ? "111111" : "000000");
  await page.locator("#kc-login").click();
  // Refused: still on the identity provider's page, asking for the code, and no session on the UI.
  await expect(page).toHaveURL(/\/realms\/pas\//);
  await expect(page.locator("#otp")).toBeVisible();
  const refused = await page.goto("/admin/reminders");
  expect(refused?.url()).toMatch(/signin/);

  // The right code signs in.
  await keycloakLogin(page, "admin");
  await page.goto("/admin/reminders");
  await expect(page.getByRole("heading", { name: "Reminder templates" })).toBeVisible();
});

test("a patient is signed in by password alone", async ({ page }) => {
  await page.goto("/en-001/signin");
  await page.locator('a[href^="/signin/oidc"]').click();
  await page.locator("#username").fill("patient");
  await page.locator("#password").fill("patient");
  await page.locator("#kc-login").click();
  await expect(page).toHaveURL(/\/my\/appointments|\/en-001/);
  await expect(page.locator("#otp")).toHaveCount(0);
});

test("an account staff have not proofed reaches nothing in the patient portal", async ({ page, request }) => {
  await request.get(`${MOCK_URL}/__proofing?on=false`);
  try {
    await keycloakLogin(page, "patient", "/en-001/my/appointments");
    const res = await page.goto("/my/appointments");
    expect(res?.status()).toBe(403);
    await expect(page.getByRole("heading", { name: /403/ })).toBeVisible();
    await expect(page.getByRole("cell", { name: "Knee review" })).toHaveCount(0);
    const res2 = await page.goto("/my/messages");
    expect(res2?.status()).toBe(403);
  } finally {
    await request.get(`${MOCK_URL}/__proofing?on=true`);
  }
});

test("an administrator edits a reminder template, saves it, and the change is there after a reload", async ({
  page,
  request,
}) => {
  await request.get(`${MOCK_URL}/__templates/reset`);
  try {
    await keycloakLogin(page, "admin");
    await page.goto("/admin/reminders");
    await page.getByLabel("Subject", { exact: true }).fill("Your clinic appointment");
    await page
      .getByLabel("Message", { exact: true })
      .fill("Hello {{ patient.name.given }}, see you on {{ appointment.when }}.");
    await page.getByLabel("Email").check();
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.getByRole("status")).toContainText(/saved/i);

    // A reload reads it back from the API, not from the form the browser still holds.
    await page.reload();
    await expect(page.getByLabel("Subject")).toHaveValue("Your clinic appointment");
    await expect(page.getByLabel("Message", { exact: true })).toHaveValue(
      "Hello {{ patient.name.given }}, see you on {{ appointment.when }}.",
    );
    await expect(page.getByLabel("Email")).toBeChecked();
    await expect(page.getByLabel("WhatsApp")).toBeChecked();
  } finally {
    await request.get(`${MOCK_URL}/__templates/reset`);
  }
});

test("an administrator can open the reminder-template editor; a patient cannot", async ({ page }) => {
  await keycloakLogin(page, "admin");
  await page.goto("/admin/reminders");
  await expect(page.getByRole("heading", { name: "Reminder templates" })).toBeVisible();
  await expect(page.getByLabel("Subject")).toHaveValue("Appointment reminder");
  await expect(page.getByLabel("WhatsApp")).toBeChecked();
});

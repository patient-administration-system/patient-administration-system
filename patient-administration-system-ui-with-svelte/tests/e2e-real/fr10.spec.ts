// FR-10 against the real PAS API on a real database (QUAL-20): what staff see on the calendar, the waiting-list
// board, the four-week schedule and in patient search is what was stored, not what a mock was told to say.
// Seeded here: a schedule and slots straight into the database (the API has no route to create them), then
// bookings and a waiting-list entry through the API, as the system would make them.
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { expect, test } from "@playwright/test";
import { keycloakLogin } from "../e2e/support/login";
import { DATABASE_URL } from "../../playwright.real-api.config";

const API = `http://127.0.0.1:${process.env.REAL_API_PORT ?? 4196}`;
const tag = `QUAL-20 ${randomUUID().slice(0, 8)}`;
// A fresh patient each run, so a retry or a second run never clashes with an earlier booking.
const person = randomUUID();
let entryId = "";

function sql(statement: string): string {
  return execFileSync("psql", [DATABASE_URL, "-v", "ON_ERROR_STOP=1", "-qtA", "-c", statement], {
    encoding: "utf8",
  }).trim();
}

async function api<T>(method: string, path: string, body: unknown): Promise<T> {
  const res = await fetch(`${API}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = (await res.json()) as { data?: T; error?: { message?: string } };
  if (!res.ok || json.data === undefined)
    throw new Error(`${method} ${path}: ${res.status} ${json.error?.message ?? ""}`);
  return json.data;
}

/** A free slot `hours` from now, 30 minutes long, on a schedule of its own. */
function slot(schedule: string, hours: number): string {
  return sql(
    `INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, created_at, updated_at)
     VALUES (gen_random_uuid(), '${schedule}', now() + interval '${hours} hours',
             now() + interval '${hours} hours 30 minutes', 'free', now(), now()) RETURNING id`,
  );
}

async function book(slotId: string, reason: string): Promise<void> {
  const appt = await api<{ id: string }>("POST", `/api/slots/${slotId}/book`, { person_id: person });
  sql(`UPDATE appointments SET reason = '${reason}' WHERE id = '${appt.id}'`);
}

test.beforeAll(async () => {
  const schedule = sql(
    `INSERT INTO schedules (id, owner_kind, owner_id, service_type, active, created_at, updated_at)
     VALUES (gen_random_uuid(), 'Practitioner', gen_random_uuid(), 'qual20', true, now(), now()) RETURNING id`,
  );
  // One soon (on the calendar's current view) and one a few days ahead (on the four-week schedule).
  await book(slot(schedule, 1), `${tag} soon`);
  await book(slot(schedule, 4 * 24), `${tag} later`);
  entryId = (
    await api<{ id: string }>("POST", "/api/waitlist", {
      person_id: person,
      target_service: "general",
      priority: "urgent",
    })
  ).id;
});

test.beforeEach(async ({ page }) => keycloakLogin(page, "admin"));

test("the calendar shows an appointment booked through the API", async ({ page }) => {
  await page.goto("/en-001/appointments");
  // The month view marks days; the agenda lists each appointment with its title.
  await page.getByRole("button", { name: "Agenda" }).click();
  await expect(page.getByText(`${tag} soon`).first()).toBeVisible();
});

test("the schedule shows the appointments of the next four weeks", async ({ page }) => {
  await page.goto("/en-001/schedule");
  await expect(page.getByText(`${tag} later`).first()).toBeVisible();
});

test("the waiting-list board shows an entry added through the API", async ({ page }) => {
  await page.goto("/en-001/waitlist?service=general");
  await expect(page.locator(`td[data-card-id='${entryId}']`)).toBeVisible();
});

test("patient search goes through the API to person-service", async ({ page }) => {
  await page.goto("/en-001/patients?q=Lovelace");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect(page.getByText("Grace Hopper")).toHaveCount(0);
});

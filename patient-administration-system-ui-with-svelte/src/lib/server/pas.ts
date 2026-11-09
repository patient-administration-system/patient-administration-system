// The BFF's client for the PAS REST API. The bearer token lives here, on the
// server; the browser only ever talks to this app's own routes.

import type { components, paths } from "./api-types";
import { PAS_API_URL, serviceToken } from "./config";

// The shapes below come from the API contract (`spec/openapi.json`) through `api-types.ts`, which
// `pnpm run types` regenerates; changing a Rust response field breaks `pnpm run check`.
type Schemas = components["schemas"];

/** The JSON body that method `M` of `P` takes, as the contract declares it: a misspelt or missing field fails
 * `pnpm run check`. */
type RequestBody<P extends keyof paths, M extends keyof paths[P]> = paths[P][M] extends {
  requestBody?: { content: { "application/json": infer B } };
}
  ? B
  : never;

export type FetchFn = typeof fetch;

export class PasError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "PasError";
  }
}

/** `{ success, data, error }` envelope the PAS wraps every response in. */
interface Envelope<T> {
  success?: boolean;
  data?: T;
  /** On a list: what it says about itself (NFR-23). */
  meta?: Schemas["PageMeta"];
  /** Set on a refusal or failure, which the PAS answers as an RFC 9457 problem. */
  code?: string;
  detail?: string;
}

export async function pas<T>(
  fetchFn: FetchFn,
  path: string,
  init: { method?: string; body?: unknown } = {},
): Promise<T> {
  return (await pasBody<T>(fetchFn, path, init)).data;
}

async function pasBody<T>(
  fetchFn: FetchFn,
  path: string,
  init: { method?: string; body?: unknown } = {},
): Promise<{ data: T; meta?: Schemas["PageMeta"] }> {
  const headers: Record<string, string> = { accept: "application/json" };
  const token = serviceToken();
  if (token) headers.authorization = `Bearer ${token}`;
  if (init.body !== undefined) headers["content-type"] = "application/json";
  let res: Response;
  try {
    res = await fetchFn(`${PAS_API_URL}${path}`, {
      method: init.method ?? "GET",
      headers,
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
  } catch {
    throw new PasError(502, "the PAS API could not be reached");
  }
  const text = await res.text();
  let env: Envelope<T> = {};
  try {
    env = text ? (JSON.parse(text) as Envelope<T>) : {};
  } catch {
    /* not JSON: fall through to the status check */
  }
  if (!res.ok) {
    // What the PAS said stays in the server log. It never travels into an error page: it can hold anything
    // (a database message, a path), and the page shows its own translated text for the status instead.
    console.error(
      `[pas] ${init.method ?? "GET"} ${path.split("?")[0]} answered ${res.status}: ${env.detail ?? text.slice(0, 200)}`,
    );
    throw new PasError(res.status, `the PAS API answered ${res.status}`);
  }
  return { data: (env.data ?? (env as unknown)) as T, meta: env.meta };
}

/** A list endpoint: anything but an array is a broken answer (502), not something for a page to trip over. */
export async function pasArray<T>(
  fetchFn: FetchFn,
  path: string,
  init: { method?: string; body?: unknown } = {},
): Promise<T[]> {
  const data = await pas<unknown>(fetchFn, path, init);
  if (!Array.isArray(data)) throw new PasError(502, "the PAS API answered with an unexpected shape");
  return data as T[];
}

/** Where a list was cut: the limit that applied when more rows exist, or `null` when it is whole. */
function cutAt(meta: Schemas["PageMeta"] | undefined): number | null {
  return meta?.truncated ? meta.limit : null;
}

/** The signed cursor of the next page, read from `meta.next` (the path and query the API gives), or `null`. */
export function nextCursor(meta: Schemas["PageMeta"] | undefined): string | null {
  if (!meta?.truncated || !meta.next) return null;
  try {
    return new URL(meta.next, "http://pas.invalid").searchParams.get("cursor");
  } catch {
    return null;
  }
}

/** A cursor as the API writes one: base64url, and short. Anything else is not forwarded. */
export function isCursor(value: string | null): value is string {
  return value !== null && /^[A-Za-z0-9_-]{1,256}$/.test(value);
}

/** `path` asking for the page that `cursor` starts, when there is one. */
export function withCursor(path: string, cursor: string | null | undefined): string {
  if (!isCursor(cursor ?? null)) return path;
  return `${path}${path.includes("?") ? "&" : "?"}cursor=${encodeURIComponent(cursor as string)}`;
}

/** A paged list: whether it was cut, so that a page can say so (NFR-23), and the cursor to the next page. */
export interface Page<T> {
  rows: T[];
  cutAt: number | null;
  next: string | null;
}

export async function pasPage<T>(fetchFn: FetchFn, path: string, cursor?: string | null): Promise<Page<T>> {
  const { data, meta } = await pasBody<unknown>(fetchFn, withCursor(path, cursor));
  if (!Array.isArray(data)) throw new PasError(502, "the PAS API answered with an unexpected shape");
  return { rows: data as T[], cutAt: cutAt(meta), next: nextCursor(meta) };
}

export type Appointment = Schemas["Appointment"];

export const patientAppointments = (f: FetchFn, personId: string) =>
  pasArray<Appointment>(f, `/api/patients/${encodeURIComponent(personId)}/appointments`);

export const appointmentsBetween = (f: FetchFn, from: string, to: string) =>
  pasArray<Appointment>(f, `/api/appointments?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`);

export type LetterTemplate = Schemas["LetterTemplate"];

export const letterTemplates = (f: FetchFn) => pasArray<LetterTemplate>(f, "/api/letter-templates");

export const updateLetterTemplate = (f: FetchFn, t: LetterTemplate) =>
  pas<LetterTemplate>(f, `/api/letter-templates/${encodeURIComponent(t.id)}`, {
    method: "PUT",
    body: t satisfies RequestBody<"/api/letter-templates/{id}", "put">,
  });

export type WaitlistEntry = Schemas["WaitlistEntry"];

export const waitlist = (f: FetchFn) => pasArray<WaitlistEntry>(f, "/api/waitlist");

export type PortalMessage = Schemas["GeneratedLetter"];

/** Messages delivered to one patient's secure portal inbox, newest first. */
export const portalMessages = (f: FetchFn, personId: string) =>
  pasArray<PortalMessage>(f, `/api/patients/${encodeURIComponent(personId)}/letters?channel=portal`);

// ---- No-show probability (CAP-12): support only, and off until people approve it --------------

export type NoShowFactor = Schemas["Factor"];

export type NoShowItem = Schemas["Shown"];

/** The stored assessments for these appointments. `active` is false while the score is switched off. */
export const noshowFor = (f: FetchFn, ids: string[]) =>
  pas<Schemas["ShownAssessments"]>(f, `/api/noshow/for-appointments?ids=${ids.map(encodeURIComponent).join(",")}`);

// ---- Reasonable adjustments (NEW-6): shown to staff beside an appointment ---------------------

export type AppointmentAdjustments = Schemas["AppointmentAdjustments"];

/** The live adjustments of each appointment's patient; only appointments whose patient has any are listed. */
export const adjustmentsFor = (f: FetchFn, ids: string[]) =>
  pasArray<AppointmentAdjustments>(
    f,
    `/api/reasonable-adjustments/for-appointments?ids=${ids.map(encodeURIComponent).join(",")}`,
  );

// ---- History (CAP-6) and long waits (CAP-8) ---------------------------------------------------

export type HistoryRow = Schemas["HistoryRow"];

/** What happened to one appointment, oldest first. */
export const appointmentHistory = (f: FetchFn, id: string) =>
  pasArray<HistoryRow>(f, `/api/appointments/${encodeURIComponent(id)}/history`);

/** Messages sent to one patient and texts received from them, newest first (never the text of a message). */
export const contactLog = (f: FetchFn, id: string, cursor?: string | null) =>
  pasPage<HistoryRow>(f, `/api/patients/${encodeURIComponent(id)}/contact-log`, cursor);

export type DueCheck = Schemas["DueCheck"];

/** Long waiters nobody has asked recently whether they still need this, longest first. */
export const dueChecks = (f: FetchFn, cursor?: string | null) =>
  pasPage<DueCheck>(f, "/api/pathways/due-check", cursor);

/** A person asks a patient whether they still need this. Nothing is removed. */
export const askStillNeeded = (f: FetchFn, pathwayId: string, askedBy: string) =>
  pas<Schemas["Asked"]>(f, `/api/pathways/${encodeURIComponent(pathwayId)}/still-needed-checks`, {
    method: "POST",
    body: { asked_by: askedBy } satisfies RequestBody<"/api/pathways/{id}/still-needed-checks", "post">,
  });

// ---- Tolerances (red / amber / green) --------------------------------------------------------

export type Rag = Schemas["Rag"];

export type ToleranceReading = Schemas["Reading"];

export type ToleranceStatus = Schemas["ToleranceStatus"];

/** A measure that went past its red limit. Open until the measure is no longer red. */
export type ToleranceAlert = Schemas["Alert"];

export const toleranceAlerts = (f: FetchFn, cursor?: string | null) =>
  pasPage<ToleranceAlert>(f, "/api/tolerance-alerts?open=true", cursor);

/** A named person says they are looking at an open alert. */
export const acknowledgeToleranceAlert = (f: FetchFn, id: string, by: string) =>
  pas<null>(f, `/api/tolerance-alerts/${encodeURIComponent(id)}/acknowledge`, {
    method: "POST",
    body: { acknowledged_by: by } satisfies RequestBody<"/api/tolerance-alerts/{id}/acknowledge", "post">,
  });

/** Each measure against its target and tolerances, and the worst tier overall. */
export async function toleranceStatus(f: FetchFn, days: number): Promise<ToleranceStatus & { cutAt: number | null }> {
  const { data, meta } = await pasBody<Partial<ToleranceStatus> | null>(
    f,
    `/api/tolerances/status?days=${encodeURIComponent(String(days))}`,
  );
  // The page reads the three lists; a body without them is a broken answer (502), not a render error.
  if (!data || ![data.readings, data.by_service, data.by_ward].every(Array.isArray)) {
    throw new PasError(502, "the PAS API answered with an unexpected shape");
  }
  return { ...(data as ToleranceStatus), cutAt: cutAt(meta) };
}

// ---- Identity proofing (DTAC-9) --------------------------------------------------------------

/** Has staff recorded checking that this sign-in account is this patient? Only a clear yes is a yes. */
export const identityProofed = async (f: FetchFn, subject: string, personId: string): Promise<boolean> =>
  (
    await pas<Schemas["Proofed"]>(
      f,
      `/api/identity-proofings/${encodeURIComponent(subject)}/check?person_id=${encodeURIComponent(personId)}`,
    )
  ).proofed === true;

// ---- Reply links (CAP-13) --------------------------------------------------------------------

export type ReplyLinkInfo = Schemas["LinkInfo"];

/** One alternative time offered after Reschedule (CAP-5). */
export type ReplyLinkOption = Schemas["LinkOption"];

export type ReplyLinkStatus = Schemas["LinkStatus"];

/** Read-only: what the link page may show. Opening a link never acts. */
export const replyLinkInfo = (f: FetchFn, token: string) =>
  pas<ReplyLinkInfo>(f, `/api/reply-links/${encodeURIComponent(token)}`);

/** A deliberate tap. The first `cancel` answers `needs_confirmation`; send it again with `confirmed`. */
export const replyLinkAct = (
  f: FetchFn,
  token: string,
  action: string,
  confirmed = false,
  barrierCode?: Schemas["BarrierCode"],
) =>
  pas<Schemas["LinkOutcome"]>(f, `/api/reply-links/${encodeURIComponent(token)}/${encodeURIComponent(action)}`, {
    method: "POST",
    body: (barrierCode ? { confirmed, barrier_code: barrierCode } : { confirmed }) satisfies RequestBody<
      "/api/reply-links/{token}/{action}",
      "post"
    >,
  });

/** The patient chose one of the times offered. Uses the link up; if the move is refused it is given back. */
export const replyLinkRebook = (f: FetchFn, token: string, slotId: string) =>
  pas<Schemas["LinkOutcome"]>(f, `/api/reply-links/${encodeURIComponent(token)}/rebook`, {
    method: "POST",
    body: { slot_id: slotId } satisfies RequestBody<"/api/reply-links/{token}/rebook", "post">,
  });

// ---- Backfill offers (PILOT-8) ---------------------------------------------------------------

export type OfferLinkInfo = Schemas["OfferInfo"];

/** Read-only: what the offer page may show. Opening a link never acts. */
export const offerLinkInfo = (f: FetchFn, token: string) =>
  pas<OfferLinkInfo>(f, `/api/offer-links/${encodeURIComponent(token)}`);

/** A deliberate tap: `accept` books the slot, `decline` offers it to the next person. */
export const offerLinkAct = (f: FetchFn, token: string, action: "accept" | "decline") =>
  pas<Schemas["OfferOutcome"]>(f, `/api/offer-links/${encodeURIComponent(token)}/${action}`, {
    method: "POST",
    body: {},
  });

// ---- Suspected conditions, patient enquiry and the ward overview (WPAS-1, WPAS-3, WPAS-4) ----

export type Suspicion = Schemas["Suspicion"];
export type SuspectedCondition = Schemas["SuspectedCondition"];
export const SUSPECTED_CONDITIONS: readonly SuspectedCondition[] = [
  "cancer",
  "obstructive_sleep_apnoea",
  "diabetes",
  "cardiovascular_disease",
  "parkinsons_disease",
  "alzheimers_disease",
];

/** Open suspected conditions, longest wait first, optionally of one condition. */
export const openSuspicions = (f: FetchFn, condition: SuspectedCondition | null, cursor?: string | null) =>
  pasPage<Suspicion>(f, condition ? `/api/suspicions?condition=${condition}` : "/api/suspicions", cursor);

export type Enquiry = Schemas["Enquiry"];

/** Everything the PAS holds for one person. The PAS audits the read. */
export const patientEnquiry = (f: FetchFn, personId: string) =>
  pas<Enquiry>(f, `/api/patients/${encodeURIComponent(personId)}/enquiry`);

export type WardOverview = Schemas["WardOverview"];

/** Every bed of a ward and who is in it. */
export const wardOverview = (f: FetchFn, wardId: string) =>
  pas<WardOverview>(f, `/api/wards/${encodeURIComponent(wardId)}/overview`);

// The BFF's client for the PAS REST API. The bearer token lives here, on the
// server; the browser only ever talks to this app's own routes.

import { PAS_API_URL, serviceToken } from "./config";

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
  error?: { code?: string; message?: string };
}

export async function pas<T>(
  fetchFn: FetchFn,
  path: string,
  init: { method?: string; body?: unknown } = {},
): Promise<T> {
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
    console.error(`[pas] ${init.method ?? "GET"} ${path.split("?")[0]} answered ${res.status}: ${env.error?.message ?? text.slice(0, 200)}`);
    throw new PasError(res.status, `the PAS API answered ${res.status}`);
  }
  return (env.data ?? (env as unknown)) as T;
}

/** A list endpoint: anything but an array is a broken answer (502), not something for a page to trip over. */
export async function pasArray<T>(fetchFn: FetchFn, path: string, init: { method?: string; body?: unknown } = {}): Promise<T[]> {
  const data = await pas<unknown>(fetchFn, path, init);
  if (!Array.isArray(data)) throw new PasError(502, "the PAS API answered with an unexpected shape");
  return data as T[];
}

export interface Appointment {
  id: string;
  person_id: string;
  start_datetime: string;
  end_datetime: string;
  status: "proposed" | "booked" | "arrived" | "fulfilled" | "cancelled" | "no_show";
  reason?: string | null;
}

export const patientAppointments = (f: FetchFn, personId: string) =>
  pasArray<Appointment>(f, `/api/patients/${encodeURIComponent(personId)}/appointments`);

export const appointmentsBetween = (f: FetchFn, from: string, to: string) =>
  pasArray<Appointment>(f, `/api/appointments?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`);

export interface LetterTemplate {
  id: string;
  name: string;
  subject: string;
  body_tera: string;
  required_variables: string[];
  channels: string[];
  active: boolean;
  /** The language of this variant, such as en-001 or cy-001. */
  locale: string;
  created_at: string;
  updated_at: string;
}

export const letterTemplates = (f: FetchFn) => pasArray<LetterTemplate>(f, "/api/letter-templates");

export const updateLetterTemplate = (f: FetchFn, t: LetterTemplate) =>
  pas<LetterTemplate>(f, `/api/letter-templates/${encodeURIComponent(t.id)}`, { method: "PUT", body: t });

export interface WaitlistEntry {
  id: string;
  person_id: string;
  target_service: string;
  priority: string;
  status: string;
}

export const waitlist = (f: FetchFn) => pasArray<WaitlistEntry>(f, "/api/waitlist");

export interface PortalMessage {
  id: string;
  rendered_subject: string;
  rendered_body: string;
  created_at: string;
}

/** Messages delivered to one patient's secure portal inbox, newest first. */
export const portalMessages = (f: FetchFn, personId: string) =>
  pasArray<PortalMessage>(f, `/api/patients/${encodeURIComponent(personId)}/letters?channel=portal`);

// ---- No-show probability (CAP-12): support only, and off until people approve it --------------

export interface NoShowFactor {
  name: string;
  detail: string;
  effect_points: number;
}

export interface NoShowItem {
  appointment_id: string;
  probability_percent?: number;
  band: "low" | "medium" | "high";
  factors: NoShowFactor[];
  sample_size: number;
}

/** The stored assessments for these appointments. `active` is false while the score is switched off. */
export const noshowFor = (f: FetchFn, ids: string[]) =>
  pas<{ active: boolean; items: NoShowItem[] }>(f, `/api/noshow/for-appointments?ids=${ids.map(encodeURIComponent).join(",")}`);

// ---- History (CAP-6) and long waits (CAP-8) ---------------------------------------------------

export interface HistoryRow {
  at: string;
  kind: string | null;
  action: string | null;
  by: string | null;
  reason_code: string | null;
}

/** What happened to one appointment, oldest first. */
export const appointmentHistory = (f: FetchFn, id: string) =>
  pasArray<HistoryRow>(f, `/api/appointments/${encodeURIComponent(id)}/history`);

/** Messages sent to one patient and texts received from them, newest first (never the text of a message). */
export const contactLog = (f: FetchFn, id: string) =>
  pasArray<HistoryRow>(f, `/api/patients/${encodeURIComponent(id)}/contact-log`);

export interface DueCheck {
  pathway_id: string;
  person_id: string;
  target_service: string;
  weeks_waited: number;
  last_check_at: string | null;
}

/** Long waiters nobody has asked recently whether they still need this, longest first. */
export const dueChecks = (f: FetchFn) => pasArray<DueCheck>(f, "/api/pathways/due-check");

/** A person asks a patient whether they still need this. Nothing is removed. */
export const askStillNeeded = (f: FetchFn, pathwayId: string, askedBy: string) =>
  pas<unknown>(f, `/api/pathways/${encodeURIComponent(pathwayId)}/still-needed-checks`, { method: "POST", body: { asked_by: askedBy } });

// ---- Tolerances (red / amber / green) --------------------------------------------------------

export type Rag = "green" | "amber" | "red" | "no_data";

export interface ToleranceReading {
  metric: string;
  kind: "time" | "money" | "resources";
  label: string;
  unit: "weeks" | "days" | "share";
  direction: "higher_is_worse" | "lower_is_worse" | "either";
  /** What the reading is about: the whole service, one service (by its type), or one ward (by its code). */
  scope_kind: "all" | "service" | "ward";
  scope_key: string;
  target: number;
  tolerance: number;
  escalate_beyond: number;
  value: number | null;
  adverse_drift: number | null;
  rag: Rag;
  sample: number;
  customised: boolean;
}

export interface ToleranceStatus {
  from: string;
  to: string;
  overall: Rag;
  readings: ToleranceReading[];
  by_service: ToleranceReading[];
  by_ward: ToleranceReading[];
}

/** A measure that went past its red limit. Open until the measure is no longer red. */
export interface ToleranceAlert {
  id: string;
  metric: string;
  scope_kind: "all" | "service" | "ward";
  scope_key: string;
  value: number;
  target: number;
  tolerance: number;
  escalate_beyond: number;
  unit: "weeks" | "days" | "share" | null;
  raised_at: string;
  acknowledged_by: string | null;
  acknowledged_at: string | null;
}

export const toleranceAlerts = (f: FetchFn) => pasArray<ToleranceAlert>(f, "/api/tolerance-alerts?open=true");

/** A named person says they are looking at an open alert. */
export const acknowledgeToleranceAlert = (f: FetchFn, id: string, by: string) =>
  pas<null>(f, `/api/tolerance-alerts/${encodeURIComponent(id)}/acknowledge`, {
    method: "POST",
    body: { acknowledged_by: by },
  });

/** Each measure against its target and tolerances, and the worst tier overall. */
export const toleranceStatus = (f: FetchFn, days: number) =>
  pas<ToleranceStatus>(f, `/api/tolerances/status?days=${encodeURIComponent(String(days))}`);

// ---- Identity proofing (DTAC-9) --------------------------------------------------------------

/** Has staff recorded checking that this sign-in account is this patient? Only a clear yes is a yes. */
export const identityProofed = async (f: FetchFn, subject: string, personId: string): Promise<boolean> =>
  (
    await pas<{ proofed: boolean }>(
      f,
      `/api/identity-proofings/${encodeURIComponent(subject)}/check?person_id=${encodeURIComponent(personId)}`,
    )
  ).proofed === true;

// ---- Reply links (CAP-13) --------------------------------------------------------------------

export interface ReplyLinkInfo {
  state: "open" | "used" | "expired";
  language: string;
  when: string;
  starts_at: string | null;
  time_zone: string;
  actions: string[];
}

/** One alternative time offered after Reschedule (CAP-5). */
export interface ReplyLinkOption {
  slot_id: string;
  starts_at: string;
  when: string;
}

export type ReplyLinkStatus =
  | "confirmed"
  | "cancelled"
  | "rebook_options"
  | "rebooked"
  | "held_for_call"
  | "reschedule_requested"
  | "needs_confirmation"
  | "help"
  | "rejected";

/** Read-only: what the link page may show. Opening a link never acts. */
export const replyLinkInfo = (f: FetchFn, token: string) =>
  pas<ReplyLinkInfo>(f, `/api/reply-links/${encodeURIComponent(token)}`);

/** A deliberate tap. The first `cancel` answers `needs_confirmation`; send it again with `confirmed`. */
export const replyLinkAct = (
  f: FetchFn,
  token: string,
  action: string,
  confirmed = false,
  barrierCode?: string,
) =>
  pas<{ status: ReplyLinkStatus; options?: ReplyLinkOption[]; time_zone: string; signpost?: string }>(
    f,
    `/api/reply-links/${encodeURIComponent(token)}/${encodeURIComponent(action)}`,
    { method: "POST", body: barrierCode ? { confirmed, barrier_code: barrierCode } : { confirmed } },
  );

/** The patient chose one of the times offered. Uses the link up; if the move is refused it is given back. */
export const replyLinkRebook = (f: FetchFn, token: string, slotId: string) =>
  pas<{ status: ReplyLinkStatus; starts_at?: string; time_zone: string }>(
    f,
    `/api/reply-links/${encodeURIComponent(token)}/rebook`,
    { method: "POST", body: { slot_id: slotId } },
  );

// ---- Backfill offers (PILOT-8) ---------------------------------------------------------------

export interface OfferLinkInfo {
  state: "open" | "used" | "expired" | "gone";
  starts_at: string | null;
  time_zone: string;
  expires_at: string | null;
  actions: string[];
}

/** Read-only: what the offer page may show. Opening a link never acts. */
export const offerLinkInfo = (f: FetchFn, token: string) =>
  pas<OfferLinkInfo>(f, `/api/offer-links/${encodeURIComponent(token)}`);

/** A deliberate tap: `accept` books the slot, `decline` offers it to the next person. */
export const offerLinkAct = (f: FetchFn, token: string, action: "accept" | "decline") =>
  pas<{ status: "accepted" | "declined" | "gone"; starts_at: string | null; time_zone: string }>(
    f,
    `/api/offer-links/${encodeURIComponent(token)}/${action}`,
    { method: "POST", body: {} },
  );

// The BFF's client for the PAS REST API. The bearer token lives here, on the
// server; the browser only ever talks to this app's own routes.

import { PAS_API_URL, serviceToken } from "./config";

type FetchFn = typeof fetch;

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
  if (!res.ok) throw new PasError(res.status, env.error?.message ?? `the PAS API answered ${res.status}`);
  return (env.data ?? (env as unknown)) as T;
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
  pas<Appointment[]>(f, `/api/patients/${encodeURIComponent(personId)}/appointments`);

export const appointmentsBetween = (f: FetchFn, from: string, to: string) =>
  pas<Appointment[]>(f, `/api/appointments?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`);

export interface LetterTemplate {
  id: string;
  name: string;
  subject: string;
  body_tera: string;
  required_variables: string[];
  channels: string[];
  active: boolean;
  created_at: string;
  updated_at: string;
}

export const letterTemplates = (f: FetchFn) => pas<LetterTemplate[]>(f, "/api/letter-templates");

export const updateLetterTemplate = (f: FetchFn, t: LetterTemplate) =>
  pas<LetterTemplate>(f, `/api/letter-templates/${encodeURIComponent(t.id)}`, { method: "PUT", body: t });

export interface WaitlistEntry {
  id: string;
  person_id: string;
  target_service: string;
  priority: string;
  status: string;
}

export const waitlist = (f: FetchFn) => pas<WaitlistEntry[]>(f, "/api/waitlist");

export interface PortalMessage {
  id: string;
  rendered_subject: string;
  rendered_body: string;
  created_at: string;
}

/** Messages delivered to one patient's secure portal inbox, newest first. */
export const portalMessages = (f: FetchFn, personId: string) =>
  pas<PortalMessage[]>(f, `/api/patients/${encodeURIComponent(personId)}/letters?channel=portal`);

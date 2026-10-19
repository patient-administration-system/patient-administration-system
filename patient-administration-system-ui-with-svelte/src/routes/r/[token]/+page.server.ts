// The page behind a link in a reminder message (CAP-13). It is public: the patient is not signed in, and
// the token in the address is the only thing that identifies the appointment.
//
// Opening it (the `load`) only READS. Email security scanners, link previews and chat apps follow links,
// so anything that acted on a GET would cancel appointments. Only a deliberate tap, which is a POST to one
// of the actions below, confirms, cancels or asks to reschedule.
import { fail } from "@sveltejs/kit";
import { isReason } from "#lib/reasons.js";
import type { Actions, PageServerLoad } from "./$types";
import { PasError, replyLinkAct, replyLinkInfo, replyLinkRebook } from "#lib/server/pas.js";

/** What the page is showing. Each is a catalogue key suffix (`link.<state>`). */
export type LinkState = "open" | "used" | "expired" | "invalid" | "unavailable";

const TOKEN = /^[0-9a-f]{64}$/;

function stateFor(e: unknown): LinkState {
  if (e instanceof PasError) {
    if (e.status === 404) return "invalid";
    if (e.status === 409) return "used";
    if (e.status === 400) return "expired";
  }
  return "unavailable";
}

export const load: PageServerLoad = async ({ params, fetch, setHeaders }) => {
  // The address carries the token: never send it to another site in a Referer, and never cache the page. It is
  // `same-origin`, not `no-referrer`: with `no-referrer` the browser sends `Origin: null` on a plain form post,
  // which SvelteKit refuses as cross-site, so a tap before the page finished loading (or with scripts off) failed.
  setHeaders({ "referrer-policy": "same-origin", "cache-control": "no-store" });
  if (!TOKEN.test(params.token)) return { state: "invalid" as LinkState, startsAt: null, timeZone: "UTC" };
  try {
    const info = await replyLinkInfo(fetch, params.token);
    return { state: info.state as LinkState, startsAt: info.starts_at, timeZone: info.time_zone };
  } catch (e) {
    return { state: stateFor(e), startsAt: null, timeZone: "UTC" };
  }
};

async function act(token: string, action: string, confirmed: boolean, f: typeof fetch, reason?: string) {
  if (!TOKEN.test(token)) return fail(404, { state: "invalid" as LinkState });
  try {
    const r = await replyLinkAct(f, token, action, confirmed, reason);
    // Reschedule offers real times first (CAP-5); the link is not used up until one is chosen.
    return {
      result: r.status,
      options: r.options ?? [],
      optionsZone: r.time_zone,
      startsAt: null as string | null,
      signpost: r.signpost ?? null,
    };
  } catch (e) {
    return fail(e instanceof PasError && e.status < 500 ? e.status : 502, { state: stateFor(e) });
  }
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

// Choosing a time. If it is refused (usually because someone else just took it) the API gives the link
// back, so the patient can ask for other times: say so rather than call the link used.
async function rebook(token: string, slotId: unknown, f: typeof fetch) {
  if (!TOKEN.test(token)) return fail(404, { state: "invalid" as LinkState });
  if (typeof slotId !== "string" || !UUID.test(slotId)) return fail(400, { state: "open" as LinkState, retry: true });
  try {
    const r = await replyLinkRebook(f, token, slotId);
    return { result: r.status, options: [], optionsZone: r.time_zone, startsAt: r.starts_at ?? null };
  } catch (e) {
    try {
      const info = await replyLinkInfo(f, token);
      if (info.state === "open") return fail(409, { state: "open" as LinkState, retry: true });
      return fail(409, { state: info.state as LinkState });
    } catch {
      return fail(e instanceof PasError && e.status < 500 ? e.status : 502, { state: stateFor(e) });
    }
  }
}

export const actions: Actions = {
  confirm: ({ params, fetch }) => act(params.token, "confirm", false, fetch),
  // The first tap on Cancel only asks "Are you sure?" and changes nothing.
  cancel: ({ params, fetch }) => act(params.token, "cancel", false, fetch),
  // The reason is optional: asking why must never stand in the way of cancelling.
  cancelSure: async ({ params, fetch, request }) => {
    const raw = (await request.formData()).get("barrier");
    const reason = isReason(raw) ? raw : undefined;
    return act(params.token, "cancel", true, fetch, reason);
  },
  reschedule: ({ params, fetch }) => act(params.token, "reschedule", false, fetch),
  help: ({ params, fetch }) => act(params.token, "help", false, fetch),
  rebook: async ({ params, fetch, request }) => rebook(params.token, (await request.formData()).get("slot_id"), fetch),
};

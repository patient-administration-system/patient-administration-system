// The page behind a link in a reminder message (CAP-13). It is public: the patient is not signed in, and
// the token in the address is the only thing that identifies the appointment.
//
// Opening it (the `load`) only READS. Email security scanners, link previews and chat apps follow links,
// so anything that acted on a GET would cancel appointments. Only a deliberate tap, which is a POST to one
// of the actions below, confirms, cancels or asks to reschedule.
import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { PasError, replyLinkAct, replyLinkInfo } from "#lib/server/pas.js";

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
  // The address carries the token: never send it onward in a Referer, and never cache the page.
  setHeaders({ "referrer-policy": "no-referrer", "cache-control": "no-store" });
  if (!TOKEN.test(params.token)) return { state: "invalid" as LinkState, startsAt: null, timeZone: "UTC" };
  try {
    const info = await replyLinkInfo(fetch, params.token);
    return { state: info.state as LinkState, startsAt: info.starts_at, timeZone: info.time_zone };
  } catch (e) {
    return { state: stateFor(e), startsAt: null, timeZone: "UTC" };
  }
};

async function act(token: string, action: string, confirmed: boolean, f: typeof fetch) {
  if (!TOKEN.test(token)) return fail(404, { state: "invalid" as LinkState });
  try {
    const r = await replyLinkAct(f, token, action, confirmed);
    return { result: r.status };
  } catch (e) {
    return fail(e instanceof PasError && e.status < 500 ? e.status : 502, { state: stateFor(e) });
  }
}

export const actions: Actions = {
  confirm: ({ params, fetch }) => act(params.token, "confirm", false, fetch),
  // The first tap on Cancel only asks "Are you sure?" and changes nothing.
  cancel: ({ params, fetch }) => act(params.token, "cancel", false, fetch),
  cancelSure: ({ params, fetch }) => act(params.token, "cancel", true, fetch),
  reschedule: ({ params, fetch }) => act(params.token, "reschedule", false, fetch),
  help: ({ params, fetch }) => act(params.token, "help", false, fetch),
};

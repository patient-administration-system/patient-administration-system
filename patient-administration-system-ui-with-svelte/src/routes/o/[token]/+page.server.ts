// The page behind a link in a "an appointment has become free" message (PILOT-8). Public: the patient is not
// signed in, and the token in the address is the only thing that identifies the offer.
//
// Opening it (the `load`) only READS; only a deliberate tap, a POST to one of the actions below, accepts or
// declines. Scanners and link previews follow links, so a GET must never take an appointment.
import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { PasError, offerLinkAct, offerLinkInfo } from "#lib/server/pas.js";

export type OfferState = "open" | "used" | "expired" | "gone" | "invalid" | "unavailable";

const TOKEN = /^[0-9a-f]{64}$/;

function stateFor(e: unknown): OfferState {
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
  if (!TOKEN.test(params.token)) return { state: "invalid" as OfferState, startsAt: null, timeZone: "UTC" };
  try {
    const info = await offerLinkInfo(fetch, params.token);
    return { state: info.state as OfferState, startsAt: info.starts_at, timeZone: info.time_zone };
  } catch (e) {
    return { state: stateFor(e), startsAt: null, timeZone: "UTC" };
  }
};

async function act(token: string, action: "accept" | "decline", f: typeof fetch) {
  if (!TOKEN.test(token)) return fail(404, { state: "invalid" as OfferState });
  try {
    const r = await offerLinkAct(f, token, action);
    return { result: r.status, startsAt: r.starts_at, timeZone: r.time_zone };
  } catch (e) {
    return fail(e instanceof PasError && e.status < 500 ? e.status : 502, { state: stateFor(e) });
  }
}

export const actions: Actions = {
  accept: ({ params, fetch }) => act(params.token, "accept", fetch),
  decline: ({ params, fetch }) => act(params.token, "decline", fetch),
};

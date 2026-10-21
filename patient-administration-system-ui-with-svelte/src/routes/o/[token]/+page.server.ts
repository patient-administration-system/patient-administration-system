// The page behind a link in a "an appointment has become free" message (PILOT-8). Public: the patient is not
// signed in, and the token in the address is the only thing that identifies the offer.
//
// Opening it (the `load`) only READS; only a deliberate tap, a POST to one of the actions below, accepts or
// declines. Scanners and link previews follow links, so a GET must never take an appointment.
import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { offerLinkAct, offerLinkInfo } from "#lib/server/pas.js";
import { LINK_PAGE_HEADERS, isLinkToken, linkFailureStatus, linkStateFor as stateFor } from "#lib/server/token-link.js";

export type OfferState = "open" | "used" | "expired" | "gone" | "invalid" | "unavailable";

export const load: PageServerLoad = async ({ params, fetch, setHeaders }) => {
  setHeaders(LINK_PAGE_HEADERS);
  if (!isLinkToken(params.token)) return { state: "invalid" as OfferState, startsAt: null, timeZone: "UTC" };
  try {
    const info = await offerLinkInfo(fetch, params.token);
    return { state: info.state as OfferState, startsAt: info.starts_at, timeZone: info.time_zone };
  } catch (e) {
    return { state: stateFor(e), startsAt: null, timeZone: "UTC" };
  }
};

async function act(token: string, action: "accept" | "decline", f: typeof fetch) {
  if (!isLinkToken(token)) return fail(404, { state: "invalid" as OfferState });
  try {
    const r = await offerLinkAct(f, token, action);
    return { result: r.status, startsAt: r.starts_at, timeZone: r.time_zone };
  } catch (e) {
    return fail(linkFailureStatus(e), { state: stateFor(e) });
  }
}

export const actions: Actions = {
  accept: ({ params, fetch }) => act(params.token, "accept", fetch),
  decline: ({ params, fetch }) => act(params.token, "decline", fetch),
};

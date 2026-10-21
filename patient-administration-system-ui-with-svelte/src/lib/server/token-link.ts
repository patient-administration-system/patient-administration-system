// What the two public token-link pages share (QUAL-21): the reply link in a reminder (`/r/{token}`, CAP-13)
// and the offer of a freed appointment (`/o/{token}`, PILOT-8). The token in the address is the only thing
// that identifies the appointment, so each page reads on GET and acts only on a deliberate POST.

import { PasError } from "./pas";

/** A link token as the PAS mints it: 64 lowercase hex characters. Anything else is never sent on. */
export const isLinkToken = (token: string): boolean => /^[0-9a-f]{64}$/.test(token);

/** What a link page shows when the PAS could not give it the link (a catalogue key suffix). */
export type LinkFault = "invalid" | "used" | "expired" | "unavailable";

/** The page state for a failed PAS call: 404 unknown, 409 already used, 400 expired, anything else unavailable. */
export function linkStateFor(e: unknown): LinkFault {
  if (e instanceof PasError) {
    if (e.status === 404) return "invalid";
    if (e.status === 409) return "used";
    if (e.status === 400) return "expired";
  }
  return "unavailable";
}

/** The status of a form failure for a failed PAS call: the PAS's own 4xx, else 502. */
export const linkFailureStatus = (e: unknown): number => (e instanceof PasError && e.status < 500 ? e.status : 502);

/**
 * Headers for a page whose address carries a token: never sent to another site in a Referer, never cached.
 * `same-origin`, not `no-referrer`: with `no-referrer` the browser sends `Origin: null` on a plain form post,
 * which SvelteKit refuses as cross-site, so a tap before the page finished loading (or with scripts off) failed.
 */
export const LINK_PAGE_HEADERS = { "referrer-policy": "same-origin", "cache-control": "no-store" } as const;

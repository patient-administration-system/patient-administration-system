// What every signed-in route does around a PAS call (QUAL-21), in one place: act as the signed-in person,
// turn a PAS failure into this app's own error page or form failure, and name the person doing something.
// The PAS's own words never reach the page: `PasError` already carries only a generic message.

import { error, fail, redirect, type ActionFailure } from "@sveltejs/kit";
import { actingAs } from "../acting-as";
import type { OidcSession } from "./oidc-session";
import { PasError, isCursor, type FetchFn, type Page } from "./pas";

type Locals = { oidc: OidcSession | null };

/** `fetch` that tells the PAS which signed-in person the call is for (`X-On-Behalf-Of`). */
export const signedInFetch = (fetch: FetchFn, locals: Locals): FetchFn => actingAs(fetch, locals.oidc?.user.sub);

/**
 * The signed-in person as an action records them: their name, else their subject, at most 120 characters.
 * Always from the session, never from a name typed into a form.
 */
export function actorName(locals: Locals): string {
  const user = locals.oidc?.user;
  return (user?.name ?? user?.sub ?? "").toString().slice(0, 120);
}

/**
 * A page's data from the PAS, or this app's error page: 502 for any PAS failure, or 404 when the PAS said
 * 404 and `notFound` is set (a record that does not exist is not an outage).
 */
export async function fromPas<T>(work: Promise<T>, opts: { notFound?: boolean } = {}): Promise<T> {
  try {
    return await work;
  } catch (e) {
    if (e instanceof PasError) error(opts.notFound && e.status === 404 ? 404 : 502, e.message);
    throw e;
  }
}

/**
 * Run an action's PAS call. `null` when it worked; otherwise the form failure to return, carrying `data`:
 * 502 when the PAS failed, 400 when it refused. Anything that is not a PAS failure is a bug and is thrown.
 */
export async function pasFailure<D extends Record<string, unknown>>(
  work: () => Promise<unknown>,
  data: D,
): Promise<ActionFailure<D> | null> {
  try {
    await work();
    return null;
  } catch (e) {
    if (e instanceof PasError) return fail(e.status >= 500 ? 502 : 400, data);
    throw e;
  }
}

/** `url` with `cursor` set, or without one: a link to another page of the same list. */
function pageHref(url: URL, cursor: string | null): string {
  const u = new URL(url);
  if (cursor) u.searchParams.set("cursor", cursor);
  else u.searchParams.delete("cursor");
  return u.pathname + u.search;
}

/** One page of a list, with links to the next page and back to the first. */
export type LinkedPage<T> = Page<T> & { nextHref: string | null; firstHref: string | null };

/**
 * A page of a list from the PAS, for the `?cursor=` in this page's own URL. A cursor that is not the shape the
 * PAS writes is not sent. One the PAS refuses (`422`: from before a restart, or for another list) sends the
 * browser back to the first page rather than to an error. Any other failure is as {@link fromPas} has it.
 */
export async function pageFromPas<T>(
  url: URL,
  load: (cursor: string | null) => Promise<Page<T>>,
  opts: { notFound?: boolean } = {},
): Promise<LinkedPage<T>> {
  const asked = url.searchParams.get("cursor");
  const cursor = isCursor(asked) ? asked : null;
  let page: Page<T>;
  try {
    page = await load(cursor);
  } catch (e) {
    if (e instanceof PasError && cursor && e.status === 422) redirect(303, pageHref(url, null));
    if (e instanceof PasError) error(opts.notFound && e.status === 404 ? 404 : 502, e.message);
    throw e;
  }
  return {
    ...page,
    nextHref: page.next ? pageHref(url, page.next) : null,
    firstHref: cursor ? pageHref(url, null) : null,
  };
}

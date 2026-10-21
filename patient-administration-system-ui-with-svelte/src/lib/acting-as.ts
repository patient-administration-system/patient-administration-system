// Who the backend-for-frontend is acting for, as the PAS API sees it (DTAC-4).
//
// The BFF authenticates to the API as a trusted *service* (its own token). The API then records
// `svc:<service> for <person>` in the audit log when the request carries `X-On-Behalf-Of`. The
// header is only honoured from an authenticated service, so a browser cannot use it to impersonate
// anyone; this wrapper is the one place the BFF sets it. Pure, so it is unit-tested.

export const ON_BEHALF_OF = "x-on-behalf-of";
const MAX_LENGTH = 200;

/** A header-safe actor id, or `undefined` when there is nothing usable. */
export function sanitizeActor(actor: string | null | undefined): string | undefined {
  if (!actor) return undefined;
  // Printable ASCII only: this travels in an HTTP header and ends up in an audit log.
  const clean = actor
    .replace(/[^\x20-\x7e]/g, "")
    .trim()
    .slice(0, MAX_LENGTH);
  return clean === "" ? undefined : clean;
}

/** `fetch`, with the acting person named on every request. Unchanged when there is no actor. */
export function actingAs(fetchFn: typeof fetch, actor: string | null | undefined): typeof fetch {
  const who = sanitizeActor(actor);
  if (!who) return fetchFn;
  return (input, init) => {
    const headers = new Headers(init?.headers);
    headers.set(ON_BEHALF_OF, who);
    return fetchFn(input, { ...init, headers });
  };
}

// One id per request, carried into the PAS API (as AXA-6 did for the operator front-end), so that a failing
// page can be followed into the API's logs. The API keeps an `X-Request-Id` it is sent and makes one when
// there is none. Pure, so it is unit-tested.

export const REQUEST_ID = "x-request-id";

/** An id as a proxy or the API writes one: letters, digits, `.`, `_` and `-`, at most 128. */
const SAFE = /^[A-Za-z0-9._-]{1,128}$/;

/** The incoming id when it is a safe shape, else a new one: a caller cannot put anything else in a log. */
export function requestIdFor(incoming: string | null | undefined): string {
  return incoming && SAFE.test(incoming) ? incoming : crypto.randomUUID();
}

/** `request` with the id added, when it is bound for the PAS API; any other request is left as it is. */
export function withRequestId(request: Request, id: string, apiUrl: string): Request {
  if (!request.url.startsWith(`${apiUrl}/`) && request.url !== apiUrl) return request;
  const headers = new Headers(request.headers);
  headers.set(REQUEST_ID, id);
  return new Request(request, { headers });
}

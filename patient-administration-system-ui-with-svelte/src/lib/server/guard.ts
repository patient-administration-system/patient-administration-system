// Route guards. Each returns the signed-in session or redirects/refuses, so a
// page can never render for the wrong person even when the nav hides the link.

import { error, redirect } from "@sveltejs/kit";
import type { OidcSession } from "./oidc-session";
import { isAdmin, isStaff, patientScope } from "../access";
import { localizePath, type Locale } from "../locales";

type Locals = { oidc: OidcSession | null; locale: Locale };

function signedIn(locals: Locals, returnTo: string): OidcSession {
  if (!locals.oidc) {
    // The page to come back to is remembered in the reader's own language.
    redirect(303, `${localizePath("/signin", locals.locale)}?return=${encodeURIComponent(localizePath(returnTo, locals.locale))}`);
  }
  return locals.oidc;
}

export function requireStaff(locals: Locals, returnTo: string): OidcSession {
  const s = signedIn(locals, returnTo);
  if (!isStaff(s.user)) error(403, "Staff access is required");
  return s;
}

export function requireAdmin(locals: Locals, returnTo: string): OidcSession {
  const s = signedIn(locals, returnTo);
  if (!isAdmin(s.user)) error(403, "Administrator access is required");
  return s;
}

/** A patient session, plus the one person id it may read. */
export function requirePatient(locals: Locals, returnTo: string): { session: OidcSession; personId: string } {
  const s = signedIn(locals, returnTo);
  const personId = patientScope(s.user);
  if (!personId) error(403, "This sign-in is not linked to a patient record");
  return { session: s, personId };
}

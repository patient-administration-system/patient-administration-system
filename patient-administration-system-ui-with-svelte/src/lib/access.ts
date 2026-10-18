// Who may see what. Pure, so it is unit-tested; the server guards and the
// navigation both read it. Roles come from the Keycloak realm (pas-realm.json).

export type Role = "patient" | "staff" | "admin";

export interface Principal {
  roles: string[];
  personId?: string;
}

export const isStaff = (p: Principal | null | undefined): boolean =>
  !!p && (p.roles.includes("staff") || p.roles.includes("admin"));

export const isAdmin = (p: Principal | null | undefined): boolean => !!p && p.roles.includes("admin");

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export const isUuid = (s: string | undefined | null): s is string => !!s && UUID.test(s);

/**
 * The person id a patient may read. A patient reads only their own record, and
 * only when the identity provider asserted a well-formed `person_id`. Staff are
 * not "a patient" here: they use the staff routes instead.
 */
export function patientScope(p: Principal | null | undefined): string | null {
  if (!p || !p.roles.includes("patient")) return null;
  return isUuid(p.personId) ? p.personId : null;
}

export interface NavItem {
  href: string;
  label: string;
}

/** Navigation for a principal. A presentation choice; every route also enforces its own guard. */
export function navFor(p: Principal | null | undefined): NavItem[] {
  if (isStaff(p)) {
    const items: NavItem[] = [
      { href: "/appointments", label: "Appointments" },
      { href: "/waitlist", label: "Waitlist" },
      { href: "/schedule", label: "Schedule" },
      { href: "/patients", label: "Patients" },
    ];
    if (isAdmin(p)) items.push({ href: "/admin/reminders", label: "Reminder templates" });
    return items;
  }
  if (p && p.roles.includes("patient")) return [
      { href: "/my/appointments", label: "My appointments" },
      { href: "/my/messages", label: "My messages" },
    ];
  return [];
}

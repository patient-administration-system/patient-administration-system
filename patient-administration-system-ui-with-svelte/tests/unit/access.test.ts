import { describe, expect, it } from "vitest";
import { isAdmin, isStaff, isUuid, navFor, patientScope } from "#lib/access.js";

const ID = "0f8fad5b-d9cb-469f-a165-70867728950e";

describe("patientScope", () => {
  it("returns the person id only for a patient with a well-formed one", () => {
    expect(patientScope({ roles: ["patient"], personId: ID })).toBe(ID);
    expect(patientScope({ roles: ["patient"], personId: "not-a-uuid" })).toBeNull();
    expect(patientScope({ roles: ["patient"] })).toBeNull();
  });
  it("never scopes staff or anonymous callers to a patient", () => {
    expect(patientScope({ roles: ["staff"], personId: ID })).toBeNull();
    expect(patientScope(null)).toBeNull();
  });
});

describe("roles", () => {
  it("admin implies staff access but staff is not admin", () => {
    expect(isStaff({ roles: ["admin"] })).toBe(true);
    expect(isAdmin({ roles: ["staff"] })).toBe(false);
    expect(isStaff({ roles: ["patient"] })).toBe(false);
  });
  it("validates uuids", () => {
    expect(isUuid(ID)).toBe(true);
    expect(isUuid("x")).toBe(false);
    expect(isUuid(undefined)).toBe(false);
  });
});

describe("isUuid", () => {
  // The whole string must be one id: found by mutation testing (QUAL-18), where dropping either anchor of the
  // pattern passed every test, so a person_id with anything before or after a valid id would have been accepted.
  it("accepts exactly one id, in either case, and nothing around it", () => {
    expect(isUuid(ID)).toBe(true);
    expect(isUuid(ID.toUpperCase())).toBe(true);
    for (const bad of [
      `x${ID}`,
      `${ID}x`,
      `../${ID}`,
      `${ID}/../other`,
      ` ${ID}`,
      `${ID}\n`,
      ID.slice(1),
      "",
      null,
      undefined,
    ]) {
      expect(isUuid(bad), String(bad)).toBe(false);
    }
  });
  it("keeps such a person_id out of a patient's scope", () => {
    expect(patientScope({ roles: ["patient"], personId: `${ID}/../${ID}` })).toBeNull();
  });
});

describe("navFor", () => {
  it("gives patients only their own pages", () => {
    expect(navFor({ roles: ["patient"] }).map((n) => n.href)).toEqual(["/my/appointments", "/my/messages"]);
    expect(navFor({ roles: ["patient"] }).map((n) => n.key)).toEqual(["nav.myAppointments", "nav.myMessages"]);
  });
  it("gives staff the history and long-wait views, and patients neither", () => {
    expect(navFor({ roles: ["staff"] }).map((n) => n.href)).toEqual(
      expect.arrayContaining(["/history", "/waits", "/tolerances"]),
    );
    expect(navFor({ roles: ["patient"] }).map((n) => n.href)).not.toContain("/history");
  });
  it("adds reminder templates for admins only", () => {
    expect(navFor({ roles: ["staff"] }).some((n) => n.href === "/admin/reminders")).toBe(false);
    expect(navFor({ roles: ["staff", "admin"] }).some((n) => n.href === "/admin/reminders")).toBe(true);
  });
  it("shows nothing when signed out", () => expect(navFor(null)).toEqual([]));
});

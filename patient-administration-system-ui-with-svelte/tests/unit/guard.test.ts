// Route guards (QUAL-19): signed out redirects to sign-in in the reader's language, the wrong role is a
// 403, and a patient reaches nothing until the PAS clearly says the account is proofed. (jsdom, like the
// other UI tests: the test build resolves SvelteKit's browser variant, whose `redirect` reads `window`.)
import { describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({ PAS_API_URL: "http://pas.test", PAS_API_TOKEN: "t", PAS_API_TOKEN_FILE: "" }));

const { requireAdmin, requirePatient, requireProofedPatient, requireStaff } = await import("#lib/server/guard.js");
const { localizePath } = await import("#lib/locales.js");

const PERSON = "3f2c1a9e-0000-4000-8000-000000000001";

function locals(user: { roles: string[]; personId?: string } | null, locale = "cy-001") {
  const oidc = user
    ? {
        id: "s",
        accessToken: "a",
        accessExpiresAt: 0,
        expiresAt: 0,
        lastSeenAt: 0,
        user: { sub: "kc-1", name: "Ada", ...user },
      }
    : null;
  return { oidc, locale } as Parameters<typeof requireStaff>[0];
}

/** What a guard threw: SvelteKit's redirect and error objects carry `status` (and `location`). */
async function thrown(f: () => unknown): Promise<{ status: number; location?: string; body?: { message: string } }> {
  try {
    await f();
  } catch (e) {
    return e as { status: number; location?: string };
  }
  throw new Error("the guard let the request through");
}

const answering = (status: number, body: unknown) =>
  vi.fn(async () => new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;

describe("signed out", () => {
  it("redirects to sign-in in the reader's language and comes back to the same page", async () => {
    for (const guard of [requireStaff, requireAdmin, requirePatient]) {
      const t = await thrown(() => guard(locals(null), "/waitlist"));
      expect(t.status).toBe(303);
      expect(t.location).toBe(
        `${localizePath("/signin", "cy-001")}?return=${encodeURIComponent(localizePath("/waitlist", "cy-001"))}`,
      );
    }
  });
});

describe("roles", () => {
  it("lets staff and administrators into staff pages, and nobody else", async () => {
    expect(requireStaff(locals({ roles: ["staff"] }), "/x").user.sub).toBe("kc-1");
    expect(requireStaff(locals({ roles: ["admin"] }), "/x").user.sub).toBe("kc-1");
    expect((await thrown(() => requireStaff(locals({ roles: ["patient"], personId: PERSON }), "/x"))).status).toBe(403);
  });

  it("keeps administrator pages for administrators", async () => {
    expect(requireAdmin(locals({ roles: ["admin"] }), "/x").user.sub).toBe("kc-1");
    expect((await thrown(() => requireAdmin(locals({ roles: ["staff"] }), "/x"))).status).toBe(403);
  });

  it("gives a patient exactly the person id in the token, and refuses one without a well-formed id", async () => {
    expect(requirePatient(locals({ roles: ["patient"], personId: PERSON }), "/x").personId).toBe(PERSON);
    expect(
      (await thrown(() => requirePatient(locals({ roles: ["patient"], personId: "not-a-uuid" }), "/x"))).status,
    ).toBe(403);
    expect((await thrown(() => requirePatient(locals({ roles: ["patient"] }), "/x"))).status).toBe(403);
    expect((await thrown(() => requirePatient(locals({ roles: ["staff"], personId: PERSON }), "/x"))).status).toBe(403);
  });
});

describe("requireProofedPatient", () => {
  const patient = () => locals({ roles: ["patient"], personId: PERSON });

  it("lets a patient in when the PAS says this account is proofed as this patient", async () => {
    const f = answering(200, { data: { proofed: true } });
    await expect(requireProofedPatient(patient(), f, "/my/appointments")).resolves.toMatchObject({ personId: PERSON });
  });

  it("refuses anything but a clear yes: no, a fault, an odd answer, an unreachable API", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const down = vi.fn(async () => {
      throw new TypeError("fetch failed");
    }) as unknown as typeof fetch;
    for (const f of [
      answering(200, { data: { proofed: false } }),
      answering(500, { error: { message: "boom" } }),
      answering(200, { data: { proofed: "yes" } }),
      down,
    ]) {
      expect((await thrown(() => requireProofedPatient(patient(), f, "/my/appointments"))).status).toBe(403);
    }
  });

  it("never asks the PAS about someone who is not a patient", async () => {
    const f = answering(200, { data: { proofed: true } });
    expect((await thrown(() => requireProofedPatient(locals({ roles: ["staff"] }), f, "/x"))).status).toBe(403);
    expect(f).not.toHaveBeenCalled();
  });
});

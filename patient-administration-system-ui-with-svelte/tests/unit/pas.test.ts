// @vitest-environment node
// The BFF's PAS client (QUAL-19): the bearer token is added on the server, every failure becomes a PasError
// with a status the page translates, and what the PAS said never travels into that error.
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("$app/env/private", () => ({
  PAS_API_URL: "http://pas.test/",
  PAS_API_TOKEN: "service-token",
  PAS_API_TOKEN_FILE: "",
}));

const {
  PasError,
  failureLine,
  routeOf,
  acknowledgeToleranceAlert,
  adjustmentsFor,
  appointmentHistory,
  appointmentsBetween,
  askStillNeeded,
  contactLog,
  dueChecks,
  facilitiesWithWards,
  identityProofed,
  isCursor,
  letterTemplates,
  nextCursor,
  noshowFor,
  offerLinkAct,
  offerLinkInfo,
  openSuspicions,
  pas,
  pasAllPages,
  pasArray,
  pasPage,
  patientAppointments,
  patientEnquiry,
  portalMessages,
  replyLinkAct,
  replyLinkInfo,
  replyLinkRebook,
  suspicionProfile,
  toleranceAlerts,
  toleranceStatus,
  updateLetterTemplate,
  waitlist,
  wardOverview,
  withCursor,
  workItems,
  workReminders,
  claimWork,
  releaseWork,
  takeNextWork,
} = await import("#lib/server/pas.js");

type Call = { url: string; init: RequestInit };

function fake(status: number, body: string): { fetch: typeof fetch; calls: Call[] } {
  const calls: Call[] = [];
  const f = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return new Response(body, { status });
  });
  return { fetch: f as unknown as typeof fetch, calls };
}

afterEach(() => vi.restoreAllMocks());

describe("what a refused call writes to the server log (PA-02)", () => {
  const token = "ab12".repeat(16);
  const person = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

  it("keeps the route and replaces every value in it", () => {
    expect(routeOf(`/api/reply-links/${token}/confirm`)).toBe("/api/reply-links/{id}/confirm");
    expect(routeOf(`/api/patients/${person}/appointments?limit=5&person=${person}`)).toBe(
      "/api/patients/{id}/appointments",
    );
    expect(routeOf("/api/hl7/v2/admit")).toBe("/api/hl7/v2/admit");
    expect(routeOf("/api/persons/Lovelace%20Ada")).toBe("/api/persons/{id}");
  });

  it("adds only a code and a request id of a safe shape", () => {
    expect(failureLine("POST", `/api/offer-links/${token}/accept`, 409, "conflict", "req-1.a_b")).toBe(
      "POST /api/offer-links/{id}/accept answered 409 code=conflict request=req-1.a_b",
    );
    expect(failureLine("GET", "/api/x", 500, "a code with words", "<script>")).toBe("GET /api/x answered 500");
  });

  it("logs neither a reply token nor a body that is not JSON", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const f = vi.fn(
      async () => new Response("Ada Lovelace, 07700 900123", { status: 502, headers: { "x-request-id": "rid-7" } }),
    ) as unknown as typeof fetch;
    await pas(f, `/api/reply-links/${token}`).catch(() => undefined);
    const line = String(log.mock.calls[0]![0]);
    expect(line).toBe("[pas] GET /api/reply-links/{id} answered 502 request=rid-7");
    expect(line).not.toContain(token);
    expect(line).not.toContain("Lovelace");
  });
});

describe("pas()", () => {
  it("calls the configured API with the service token and returns the envelope's data", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ success: true, data: { id: "a1" } }));
    await expect(pas(fetch, "/api/x?y=1")).resolves.toEqual({ id: "a1" });
    expect(calls[0]!.url).toBe("http://pas.test/api/x?y=1");
    const h = calls[0]!.init.headers as Record<string, string>;
    expect(h.authorization).toBe("Bearer service-token");
    expect(h["content-type"]).toBeUndefined();
  });

  it("sends a JSON body with its content type", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ data: {} }));
    await pas(fetch, "/api/x", { method: "PUT", body: { a: 1 } });
    expect(calls[0]!.init.method).toBe("PUT");
    expect(calls[0]!.init.body).toBe('{"a":1}');
    expect((calls[0]!.init.headers as Record<string, string>)["content-type"]).toBe("application/json");
  });

  it("turns an unreachable API into a 502", async () => {
    const f = vi.fn(async () => {
      throw new TypeError("connect ECONNREFUSED 10.0.0.5:8080");
    }) as unknown as typeof fetch;
    const e = await pas(f, "/api/x").catch((x: unknown) => x);
    expect(e).toBeInstanceOf(PasError);
    expect((e as InstanceType<typeof PasError>).status).toBe(502);
    expect((e as Error).message).not.toContain("10.0.0.5");
  });

  it("keeps the API's status and puts neither its words nor the path's values in the log", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const secret = 'relation "patients" does not exist at /srv/pas/src/db.rs:42';
    const { fetch } = fake(
      500,
      JSON.stringify({ type: "about:blank", title: "Internal Server Error", status: 500, code: "DB", detail: secret }),
    );
    const e = (await pas(fetch, "/api/x?person=abc").catch((x: unknown) => x)) as InstanceType<typeof PasError>;
    expect(e).toBeInstanceOf(PasError);
    expect(e.status).toBe(500);
    expect(e.message).toBe("the PAS API answered 500");
    expect(log).toHaveBeenCalledOnce();
    const line = String(log.mock.calls[0]![0]);
    // PA-02: the API's words can carry personal data; the log has the status and the code, not the detail.
    expect(line).not.toContain("patients");
    expect(line).toContain("answered 500");
    expect(line).toContain("code=DB");
    // The query string (which can hold an identifier) is not logged.
    expect(line).not.toContain("person=abc");
  });

  it("does not trip over an error that is not JSON", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { fetch } = fake(503, "<html>Service Unavailable</html>");
    await expect(pas(fetch, "/api/x")).rejects.toMatchObject({ status: 503 });
  });

  it("accepts a successful response with an empty body", async () => {
    await expect(pas(fake(200, "").fetch, "/api/x")).resolves.toEqual({});
  });
});

describe("pasArray()", () => {
  it("returns a list", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: [1, 2] }));
    await expect(pasArray(fetch, "/api/x")).resolves.toEqual([1, 2]);
  });

  it("treats anything but a list as a broken answer (502)", async () => {
    for (const body of [JSON.stringify({ data: { not: "a list" } }), JSON.stringify({ data: null }), "not json"]) {
      const { fetch } = fake(200, body);
      await expect(pasArray(fetch, "/api/x"), body).rejects.toMatchObject({ status: 502 });
    }
  });
});

describe("pasPage()", () => {
  it("says where a list was cut, and null for a whole one or a body with no meta", async () => {
    const page = (meta: unknown) => pasPage(fake(200, JSON.stringify({ data: [1], meta })).fetch, "/list");
    await expect(page({ limit: 1, returned: 1, truncated: true })).resolves.toEqual({
      rows: [1],
      cutAt: 1,
      next: null,
    });
    await expect(page({ limit: 5, returned: 1, truncated: false })).resolves.toEqual({
      rows: [1],
      cutAt: null,
      next: null,
    });
    await expect(page(undefined)).resolves.toEqual({ rows: [1], cutAt: null, next: null });
  });

  it("gives the cursor of the next page, and asks for a page with the cursor it is given", async () => {
    const meta = { limit: 2, returned: 2, truncated: true, next: "/api/list?open=true&limit=2&cursor=AbC_-9" };
    const { fetch, calls } = fake(200, JSON.stringify({ data: [1, 2], meta }));
    await expect(pasPage(fetch, "/api/list?open=true", "Xy_-1")).resolves.toEqual({
      rows: [1, 2],
      cutAt: 2,
      next: "AbC_-9",
    });
    expect(calls[0]?.url).toBe("http://pas.test/api/list?open=true&cursor=Xy_-1");
  });

  it("never sends a cursor of the wrong shape", async () => {
    for (const bad of ["", "../admin", "a b", "a&limit=500", "x".repeat(257)]) {
      expect(isCursor(bad), bad).toBe(false);
      expect(withCursor("/api/list", bad), bad).toBe("/api/list");
    }
    expect(withCursor("/api/list", null)).toBe("/api/list");
    expect(withCursor("/api/list", "ok_-1")).toBe("/api/list?cursor=ok_-1");
    expect(withCursor("/api/list?a=1", "ok")).toBe("/api/list?a=1&cursor=ok");
  });

  it("has no next cursor for a whole list, or a next without one", () => {
    expect(nextCursor(undefined)).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: false, next: "/x?cursor=a" })).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: true })).toBeNull();
    expect(nextCursor({ limit: 1, returned: 1, truncated: true, next: "/x?limit=1" })).toBeNull();
  });

  it("treats a body that is not a list as a broken answer", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: {} }));
    await expect(pasPage(fetch, "/list")).rejects.toMatchObject({ status: 502 });
  });
});

describe("pasAllPages()", () => {
  it("joins every signed-cursor page so a busy calendar is whole", async () => {
    const calls: string[] = [];
    const fetch = vi.fn(async (url: RequestInfo | URL) => {
      calls.push(String(url));
      const second = String(url).includes("cursor=next_2");
      return new Response(
        JSON.stringify(
          second
            ? { data: [{ id: "a3" }], meta: { limit: 2, returned: 1, truncated: false } }
            : {
                data: [{ id: "a1" }, { id: "a2" }],
                meta: {
                  limit: 2,
                  returned: 2,
                  truncated: true,
                  next: "/api/appointments?from=x&to=y&limit=2&cursor=next_2",
                },
              },
        ),
        { status: 200 },
      );
    }) as unknown as typeof globalThis.fetch;

    await expect(pasAllPages<{ id: string }>(fetch, "/api/appointments?from=x&to=y&limit=2")).resolves.toEqual([
      { id: "a1" },
      { id: "a2" },
      { id: "a3" },
    ]);
    expect(calls).toEqual([
      "http://pas.test/api/appointments?from=x&to=y&limit=2",
      "http://pas.test/api/appointments?from=x&to=y&limit=2&cursor=next_2",
    ]);
  });

  it("refuses a repeated cursor instead of looping or showing a partial calendar", async () => {
    const fetch = fake(
      200,
      JSON.stringify({
        data: [{ id: "a1" }],
        meta: { limit: 1, returned: 1, truncated: true, next: "/api/x?cursor=again" },
      }),
    ).fetch;
    await expect(pasAllPages(fetch, "/api/x")).rejects.toMatchObject({ status: 502 });
  });

  it("refuses a truncated page with no cursor instead of showing a partial calendar", async () => {
    const fetch = fake(
      200,
      JSON.stringify({ data: [{ id: "a1" }], meta: { limit: 1, returned: 1, truncated: true } }),
    ).fetch;
    await expect(pasAllPages(fetch, "/api/x")).rejects.toMatchObject({ status: 502 });
  });

  it("asks for the largest allowed pages when reading appointments", async () => {
    const { fetch, calls } = fake(
      200,
      JSON.stringify({ data: [], meta: { limit: 500, returned: 0, truncated: false } }),
    );
    await appointmentsBetween(fetch, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z");
    expect(calls[0]?.url).toBe(
      "http://pas.test/api/appointments?from=2026-01-01T00%3A00%3A00Z&to=2026-02-01T00%3A00%3A00Z&limit=500",
    );
  });
});

describe("toleranceStatus()", () => {
  const good = { overall: "green", from: "a", to: "b", readings: [], by_service: [], by_ward: [] };

  it("returns a status that has its three lists", async () => {
    const { fetch } = fake(200, JSON.stringify({ data: good }));
    await expect(toleranceStatus(fetch, 30)).resolves.toEqual({ ...good, cutAt: null });
  });

  it("says where its lists were cut", async () => {
    const meta = { limit: 500, returned: 500, truncated: true };
    const { fetch } = fake(200, JSON.stringify({ data: good, meta }));
    await expect(toleranceStatus(fetch, 30)).resolves.toEqual({ ...good, cutAt: 500 });
  });

  it("treats a body without them as a broken answer (502), so the page never renders it", async () => {
    for (const data of [null, {}, { ...good, readings: undefined }, { ...good, by_ward: "none" }, [1]]) {
      const { fetch } = fake(200, JSON.stringify({ data }));
      await expect(toleranceStatus(fetch, 30), JSON.stringify(data)).rejects.toMatchObject({ status: 502 });
    }
  });
});

describe("identityProofed()", () => {
  it("is true only for an explicit proofed: true", async () => {
    for (const [data, want] of [
      [{ proofed: true }, true],
      [{ proofed: "true" }, false],
      [{ proofed: 1 }, false],
      [{}, false],
    ] as const) {
      const { fetch, calls } = fake(200, JSON.stringify({ data }));
      await expect(identityProofed(fetch, "kc|sub/1", "p 1"), JSON.stringify(data)).resolves.toBe(want);
      expect(calls[0]!.url).toBe("http://pas.test/api/identity-proofings/kc%7Csub%2F1/check?person_id=p%201");
    }
  });
});

describe("facilitiesWithWards()", () => {
  it("asks for each facility's wards and keeps only the active ones", async () => {
    const calls: string[] = [];
    const f = vi.fn(async (url: RequestInfo | URL) => {
      calls.push(String(url));
      const data = String(url).endsWith("/api/facilities")
        ? [{ id: "f/1", name: "General" }]
        : [
            { id: "w1", name: "Ward 1", active: true },
            { id: "w2", name: "Shut", active: false },
          ];
      return new Response(JSON.stringify({ data }), { status: 200 });
    }) as unknown as typeof fetch;
    const got = await facilitiesWithWards(f);
    expect(calls).toEqual(["http://pas.test/api/facilities", "http://pas.test/api/facilities/f%2F1/wards"]);
    expect(got).toHaveLength(1);
    expect(got[0]!.facility.name).toBe("General");
    expect(got[0]!.wards.map((w) => w.id)).toEqual(["w1"]);
  });
});

describe("typed PAS route wrappers", () => {
  it("encode path and query values and use the declared action method", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ data: [] }));
    await patientAppointments(fetch, "person/1");
    await letterTemplates(fetch);
    await waitlist(fetch);
    await portalMessages(fetch, "person/1");
    await noshowFor(fetch, ["a/b", "c d"]);
    await adjustmentsFor(fetch, ["a/b", "c d"]);
    await appointmentHistory(fetch, "appointment/1");
    await contactLog(fetch, "person/1", "page_2");
    await dueChecks(fetch, "page_3");
    await askStillNeeded(fetch, "pathway/1", "Ada");
    await toleranceAlerts(fetch, "page_4");
    await acknowledgeToleranceAlert(fetch, "alert/1", "Ada");
    await replyLinkInfo(fetch, "token/1");
    await replyLinkAct(fetch, "token/1", "cancel now", true, "transport");
    await replyLinkRebook(fetch, "token/1", "slot/1");
    await offerLinkInfo(fetch, "offer/1");
    await offerLinkAct(fetch, "offer/1", "accept");
    await suspicionProfile(fetch);
    await openSuspicions(fetch, "type/a", "page_5");
    await patientEnquiry(fetch, "person/1");
    await wardOverview(fetch, "ward/1");

    expect(calls.map((c) => [c.init.method, c.url, c.init.body])).toEqual([
      ["GET", "http://pas.test/api/patients/person%2F1/appointments", undefined],
      ["GET", "http://pas.test/api/letter-templates", undefined],
      ["GET", "http://pas.test/api/waitlist", undefined],
      ["GET", "http://pas.test/api/patients/person%2F1/letters?channel=portal", undefined],
      ["GET", "http://pas.test/api/noshow/for-appointments?ids=a%2Fb,c%20d", undefined],
      ["GET", "http://pas.test/api/reasonable-adjustments/for-appointments?ids=a%2Fb,c%20d", undefined],
      ["GET", "http://pas.test/api/appointments/appointment%2F1/history", undefined],
      ["GET", "http://pas.test/api/patients/person%2F1/contact-log?cursor=page_2", undefined],
      ["GET", "http://pas.test/api/pathways/due-check?cursor=page_3", undefined],
      ["POST", "http://pas.test/api/pathways/pathway%2F1/still-needed-checks", '{"asked_by":"Ada"}'],
      ["GET", "http://pas.test/api/tolerance-alerts?open=true&cursor=page_4", undefined],
      ["POST", "http://pas.test/api/tolerance-alerts/alert%2F1/acknowledge", '{"acknowledged_by":"Ada"}'],
      ["GET", "http://pas.test/api/reply-links/token%2F1", undefined],
      [
        "POST",
        "http://pas.test/api/reply-links/token%2F1/cancel%20now",
        '{"confirmed":true,"barrier_code":"transport"}',
      ],
      ["POST", "http://pas.test/api/reply-links/token%2F1/rebook", '{"slot_id":"slot/1"}'],
      ["GET", "http://pas.test/api/offer-links/offer%2F1", undefined],
      ["POST", "http://pas.test/api/offer-links/offer%2F1/accept", "{}"],
      ["GET", "http://pas.test/api/suspicion-profile", undefined],
      ["GET", "http://pas.test/api/suspicions?condition=type%2Fa&cursor=page_5", undefined],
      ["GET", "http://pas.test/api/patients/person%2F1/enquiry", undefined],
      ["GET", "http://pas.test/api/wards/ward%2F1/overview", undefined],
    ]);
  });

  it("takes the optional wrapper branches", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ data: [] }));
    await replyLinkAct(fetch, "token", "cancel");
    await openSuspicions(fetch, null);
    await updateLetterTemplate(fetch, { id: "template/1" } as Parameters<typeof updateLetterTemplate>[1]);
    expect(nextCursor({ limit: 1, returned: 1, truncated: true, next: "http://[" })).toBeNull();
    expect(calls.map((c) => [c.init.method, c.url, c.init.body])).toEqual([
      ["POST", "http://pas.test/api/reply-links/token/cancel", '{"confirmed":false}'],
      ["GET", "http://pas.test/api/suspicions", undefined],
      ["PUT", "http://pas.test/api/letter-templates/template%2F1", '{"id":"template/1"}'],
    ]);
  });
});

describe("the work queue (TSC-1)", () => {
  const item = { id: "w1", kind: "call", status: "open", owner: null };

  it("asks for a list with only the filters given", async () => {
    const { fetch, calls } = fake(
      200,
      JSON.stringify({ data: [item], meta: { limit: 200, returned: 1, truncated: false } }),
    );
    const page = await workItems(fetch, {});
    expect(calls[0]!.url).toBe("http://pas.test/api/work-items");
    expect(page.rows).toEqual([item]);
    await workItems(fetch, { queue: "booking", owner: "me", overdue: true });
    expect(calls[1]!.url).toBe("http://pas.test/api/work-items?queue=booking&owner=me&overdue=true");
    await workItems(fetch, { owner: "none" }, "c1");
    expect(calls[2]!.url).toContain("/api/work-items?owner=none");
    expect(calls[2]!.url).toContain("cursor=c1");
  });

  it("reads the signed-in person's reminders", async () => {
    const r = { overdue: [item], due_today: [], due_by: "2026-11-18T17:00:00Z" };
    const { fetch, calls } = fake(200, JSON.stringify({ data: r }));
    await expect(workReminders(fetch)).resolves.toEqual(r);
    expect(calls[0]!.url).toBe("http://pas.test/api/work-items/reminders");
  });

  it("claims, releases and takes the next item with a POST, escaping the id and queue", async () => {
    const { fetch, calls } = fake(200, JSON.stringify({ data: item }));
    await claimWork(fetch, "a/b");
    await releaseWork(fetch, "w1");
    await takeNextWork(fetch, "operations");
    expect(calls.map((c) => [c.init.method, c.url])).toEqual([
      ["POST", "http://pas.test/api/work-items/a%2Fb/claim"],
      ["POST", "http://pas.test/api/work-items/w1/release"],
      ["POST", "http://pas.test/api/work-queues/operations/next"],
    ]);
  });

  it("turns a refusal into a PasError with the PAS's status", async () => {
    const { fetch } = fake(409, JSON.stringify({ code: "conflict", detail: "someone else has this item" }));
    const e = await claimWork(fetch, "w1").catch((x: unknown) => x);
    expect(e).toBeInstanceOf(PasError);
    expect((e as InstanceType<typeof PasError>).status).toBe(409);
  });
});

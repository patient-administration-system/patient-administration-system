import { describe, expect, it } from "vitest";
import {
  appointmentEvents,
  appointmentGantt,
  ganttRange,
  patientRows,
  personName,
  statusKey,
  waitlistCards,
  waitlistColumns,
} from "#lib/views.js";
import { translate } from "#lib/i18n.svelte.js";

// English and Welsh stand-ins for the reactive `t`, so these stay pure.
const en = (key: string, values?: Record<string, string | number>) => translate(key, "en-001", values);
const cy = (key: string, values?: Record<string, string | number>) => translate(key, "cy-001", values);

const appt = (id: string, start: string, status = "booked", reason: string | null = null) => ({
  id,
  person_id: "p1",
  start_datetime: start,
  end_datetime: start.replace(/T(\d\d)/, (_, h) => `T${String(Number(h) + 1).padStart(2, "0")}`),
  status,
  reason,
});

describe("appointmentEvents", () => {
  it("maps to one-day calendar events with time and status", () => {
    const [e] = appointmentEvents([appt("a", "2026-10-20T09:30:00Z", "booked", "Check-up")], en);
    expect(e).toMatchObject({ id: "a", date: "2026-10-20", title: "09:30 Check-up", done: false });
    expect(e?.values).toEqual({ time: "09:30–10:30", status: "Booked" });
  });
  it("speaks the reader's language for the status and the generic title", () => {
    const [e] = appointmentEvents([appt("a", "2026-10-20T09:30:00Z", "booked")], cy);
    expect(e?.title).toBe("09:30 Apwyntiad");
    expect(e?.values).toEqual({ time: "09:30–10:30", status: "Wedi'i archebu" });
  });
  it("marks fulfilled as done and falls back to a generic title", () => {
    const [e] = appointmentEvents([appt("a", "2026-10-20T09:30:00Z", "fulfilled")], en);
    expect(e?.done).toBe(true);
    expect(e?.title).toBe("09:30 Appointment");
  });
});

describe("statusKey", () => {
  it("names a catalogue entry, and did-not-attend is words in English", () => {
    expect(statusKey("no_show")).toBe("status.no_show");
    expect(en(statusKey("no_show"))).toBe("Did not attend");
  });
});

describe("appointmentGantt", () => {
  it("groups appointments under one parent per day, in time order", () => {
    const tasks = appointmentGantt([
      appt("b", "2026-10-20T11:00:00Z"),
      appt("a", "2026-10-20T09:00:00Z"),
      appt("c", "2026-10-21T09:00:00Z"),
    ], en);
    expect(tasks.filter((t) => !t.parentId).map((t) => t.id)).toEqual(["day:2026-10-20", "day:2026-10-21"]);
    expect(tasks.filter((t) => t.parentId === "day:2026-10-20").map((t) => t.id)).toEqual(["a", "b"]);
  });
});

describe("ganttRange", () => {
  it("covers at least a week from today", () => {
    expect(ganttRange([], "2026-10-18")).toEqual({ start: "2026-10-18", end: "2026-10-24" });
  });
  it("stretches to the last appointment and back to an earlier one", () => {
    const r = ganttRange([appt("a", "2026-10-10T09:00:00Z"), appt("b", "2026-11-02T09:00:00Z")], "2026-10-18");
    expect(r).toEqual({ start: "2026-10-10", end: "2026-11-02" });
  });
});

describe("waitlistCards", () => {
  it("orders by clinical priority and files unknown statuses under waiting", () => {
    const cards = waitlistCards([
      { id: "1", person_id: "aaaaaaaa-1", target_service: "ENT", priority: "routine", status: "waiting" },
      { id: "2", person_id: "bbbbbbbb-2", target_service: "ENT", priority: "emergency", status: "mystery" },
    ], en);
    expect(cards.map((c) => c.id)).toEqual(["2", "1"]);
    expect(cards[0]).toMatchObject({ columnId: "waiting" });
    // WCAG 2.5.3: all visible text is in the title, so the cell's accessible name contains it.
    expect(cards[0]?.title).toMatch(/^ENT · Patient .*bbbbbbbb.* · Emergency$/);
    expect(cards[0]?.subtitle).toBeUndefined();
    expect(cards[0]?.badges).toBeUndefined();
  });
});

describe("waitlistColumns", () => {
  it("titles each column from the catalogue", () => {
    expect(waitlistColumns(en).map((c) => c.title)).toEqual(["Waiting", "Booked", "Treated", "Removed"]);
    expect(waitlistColumns(cy)[0]?.title).toBe("Yn aros");
  });
});

describe("personName / patientRows", () => {
  it("handles strings, objects and arrays", () => {
    expect(personName({ name: "Ada Lovelace" })).toBe("Ada Lovelace");
    expect(personName({ name: { given: ["Ada", "A"], family: "Lovelace" } })).toBe("Ada A Lovelace");
    expect(personName({ name: [{ given: "Grace", family: "Hopper" }] })).toBe("Grace Hopper");
    expect(personName({})).toBe("");
  });
  it("maps rows for the grid", () => {
    expect(patientRows([{ id: "x", name: "A B", birth_date: "1990-01-02" }])).toEqual([
      { id: "x", name: "A B", birthDate: "1990-01-02" },
    ]);
  });
});

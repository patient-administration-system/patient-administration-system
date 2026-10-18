import { describe, expect, it } from "vitest";
import {
  appointmentEvents,
  appointmentGantt,
  ganttRange,
  patientRows,
  personName,
  statusLabel,
  waitlistCards,
} from "#lib/views.js";

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
    const [e] = appointmentEvents([appt("a", "2026-10-20T09:30:00Z", "booked", "Check-up")]);
    expect(e).toMatchObject({ id: "a", date: "2026-10-20", title: "09:30 Check-up", done: false });
    expect(e?.values).toEqual({ time: "09:30–10:30", status: "Booked" });
  });
  it("marks fulfilled as done and falls back to a generic title", () => {
    const [e] = appointmentEvents([appt("a", "2026-10-20T09:30:00Z", "fulfilled")]);
    expect(e?.done).toBe(true);
    expect(e?.title).toBe("09:30 Appointment");
  });
});

describe("statusLabel", () => {
  it("says did-not-attend in words", () => expect(statusLabel("no_show")).toBe("Did not attend"));
});

describe("appointmentGantt", () => {
  it("groups appointments under one parent per day, in time order", () => {
    const tasks = appointmentGantt([
      appt("b", "2026-10-20T11:00:00Z"),
      appt("a", "2026-10-20T09:00:00Z"),
      appt("c", "2026-10-21T09:00:00Z"),
    ]);
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
    ]);
    expect(cards.map((c) => c.id)).toEqual(["2", "1"]);
    expect(cards[0]).toMatchObject({ columnId: "waiting", badges: ["Emergency"] });
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

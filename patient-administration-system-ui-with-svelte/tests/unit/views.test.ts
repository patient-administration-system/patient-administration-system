import { describe, expect, it } from "vitest";
import {
  appointmentEvents,
  noshowText,
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
    const tasks = appointmentGantt(
      [appt("b", "2026-10-20T11:00:00Z"), appt("a", "2026-10-20T09:00:00Z"), appt("c", "2026-10-21T09:00:00Z")],
      en,
    );
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
    const cards = waitlistCards(
      [
        { id: "1", person_id: "aaaaaaaa-1", target_service: "ENT", priority: "routine", status: "waiting" },
        { id: "2", person_id: "bbbbbbbb-2", target_service: "ENT", priority: "emergency", status: "mystery" },
      ],
      en,
    );
    expect(cards.map((c) => c.id)).toEqual(["2", "1"]);
    expect(cards[0]).toMatchObject({ columnId: "waiting", title: "ENT", badges: ["Emergency"] });
    expect(cards[0]?.subtitle).toMatch(/^Patient .*bbbbbbbb/);
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

describe("the no-show support flag (CAP-12)", () => {
  const catalogue: Record<string, string> = {
    "noshow.band.high": "High",
    "noshow.band.low": "Low",
    "noshow.band.medium": "Medium",
    "noshow.sample": "Based on {n} past visits",
    "noshow.points": "{points} points",
    "noshow.factor.past_visits": "Past visits",
    "noshow.factor.lead_time": "Lead time",
    "noshow.factor.age_band": "Age band",
    "noshow.factor.day_and_time": "Day and time",
    "noshow.factor.base_rate": "Base rate",
  };
  const tt = (k: string, v?: Record<string, string | number>) =>
    (catalogue[k] ?? k).replace(/\{(\w+)\}/g, (_, n) => String(v?.[n] ?? ""));
  const flag = {
    appointment_id: "a",
    band: "high" as const,
    sample_size: 4,
    factors: [
      { name: "base_rate", effect_points: 10 },
      { name: "past_visits", effect_points: 62 },
      { name: "lead_time", effect_points: 2 },
      { name: "age_band", effect_points: 0 },
    ],
  };

  it("says the band in words and with a shape, never colour alone, with the reasons", () => {
    const text = noshowText(flag, tt);
    expect(text).toContain("▲ High.");
    expect(text).toContain("Based on 4 past visits.");
    expect(text).toContain("Past visits");
    expect(text).toContain("Lead time");
    expect(text, "no number while the weights are uncalibrated").not.toMatch(/\d+%|points/);
    expect(text, "a factor that did not move it is not listed").not.toContain("Age band");
    expect(text, "the base rate is not a reason").not.toContain("Base rate");
  });

  it("shows the percentage and the point sizes only once the server sends them (calibrated)", () => {
    const text = noshowText({ ...flag, probability_percent: 74 }, tt);
    expect(text).toContain("▲ High, 74%.");
    expect(text).toContain("Past visits +62 points");
  });

  it("uses a different shape for each band", () => {
    const shapes = (["low", "medium", "high"] as const).map((band) => noshowText({ ...flag, band }, tt).split(" ")[0]);
    expect(new Set(shapes).size).toBe(3);
  });

  it("adds a support value only to the appointment that has a flag, and leaves the others as they were", () => {
    const rows = [appt("a", "2026-10-20T09:30:00Z", "booked"), appt("b", "2026-10-21T09:30:00Z", "booked")];
    const [a, b] = appointmentEvents(rows, tt, [flag]);
    expect(a.values?.support).toContain("High");
    expect(b.values && "support" in b.values).toBe(false);
    // With no flags at all the events are what they were before the score existed.
    expect(appointmentEvents(rows, tt).every((e) => !(e.values && "support" in e.values))).toBe(true);
  });
});

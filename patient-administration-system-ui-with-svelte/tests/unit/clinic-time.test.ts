import { describe, expect, it } from "vitest";
import { appointmentEvents, appointmentGantt, dayOf, knownZone, startOfDayIn, timeOf, todayIn } from "#lib/views.js";

// PA-13: the portal shows an appointment on the clinic's clock, as the reminder does, whatever the zone of the
// server or the browser. Europe/London is the default clinic zone (PAS_TIMEZONE).
const LONDON = "Europe/London";
const t = (key: string) => key;

describe("an appointment on the clinic's clock", () => {
  it("is an hour ahead of UTC in summer and the same in winter", () => {
    expect([dayOf("2026-07-15T08:30:00Z", LONDON), timeOf("2026-07-15T08:30:00Z", LONDON)]).toEqual([
      "2026-07-15",
      "09:30",
    ]);
    expect([dayOf("2026-01-15T08:30:00Z", LONDON), timeOf("2026-01-15T08:30:00Z", LONDON)]).toEqual([
      "2026-01-15",
      "08:30",
    ]);
  });

  it("falls on the next day when it is after midnight on the clinic's clock but not in UTC", () => {
    expect(dayOf("2026-07-15T23:30:00Z", LONDON)).toBe("2026-07-16");
    expect(timeOf("2026-07-15T23:30:00Z", LONDON)).toBe("00:30");
    expect(dayOf("2026-07-15T23:30:00Z")).toBe("2026-07-15");
  });

  it("follows both clock changes", () => {
    // Spring forward at 01:00 UTC on 29 March 2026; fall back at 01:00 UTC on 25 October 2026.
    expect(timeOf("2026-03-29T00:30:00Z", LONDON)).toBe("00:30");
    expect(timeOf("2026-03-29T01:30:00Z", LONDON)).toBe("02:30");
    expect(timeOf("2026-10-25T00:30:00Z", LONDON)).toBe("01:30");
    expect(timeOf("2026-10-25T01:30:00Z", LONDON)).toBe("01:30");
  });

  it("puts the calendar cell and the list on the same day and time", () => {
    const appt = {
      id: "a",
      person_id: "p",
      start_datetime: "2026-07-15T23:30:00Z",
      end_datetime: "2026-07-16T00:00:00Z",
      status: "booked",
      reason: "Check-up",
    };
    const [event] = appointmentEvents([appt], t, [], [], LONDON);
    expect(event.date).toBe(dayOf(appt.start_datetime, LONDON));
    expect(event.title).toBe("00:30 Check-up");
    expect(event.values?.time).toBe("00:30–01:00");
    const [day, task] = appointmentGantt([appt], t, LONDON);
    expect([day.start, task.start, task.label]).toEqual(["2026-07-16", "2026-07-16", "00:30 Check-up"]);
  });
});

describe("today on the clinic's clock", () => {
  it("is the clinic's date, not the server's", () => {
    expect(todayIn(LONDON, Date.parse("2026-07-15T23:30:00Z"))).toBe("2026-07-16");
    expect(todayIn("UTC", Date.parse("2026-07-15T23:30:00Z"))).toBe("2026-07-15");
  });

  it("begins at the clinic's midnight, on ordinary days and on the days the clocks change", () => {
    const start = (now: string) => startOfDayIn(LONDON, Date.parse(now)).toISOString();
    expect(start("2026-07-15T12:00:00Z")).toBe("2026-07-14T23:00:00.000Z");
    expect(start("2026-01-15T12:00:00Z")).toBe("2026-01-15T00:00:00.000Z");
    expect(start("2026-03-29T12:00:00Z")).toBe("2026-03-29T00:00:00.000Z");
    expect(start("2026-10-25T12:00:00Z")).toBe("2026-10-24T23:00:00.000Z");
  });
});

describe("the clinic zone setting", () => {
  it("accepts a zone the runtime knows and refuses anything else", () => {
    expect(knownZone(" Europe/London ")).toBe(LONDON);
    expect(knownZone("Not/A_Zone")).toBeNull();
    expect(knownZone("")).toBeNull();
    expect(knownZone(undefined)).toBeNull();
  });
});

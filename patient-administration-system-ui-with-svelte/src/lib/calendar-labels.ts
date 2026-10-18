// The labels the Lily calendar shows, from the catalogue and `Intl` (never hard-coded English).
// Pure, so it is unit-tested; `t` is passed in.

import type { CalendarView, CalendarViewLabels } from "@lilydesignsystem/svelte-calendar";
import type { Translate } from "./views.js";

const VIEW_KEY: Record<CalendarView, string> = {
  day: "calendar.day",
  week: "calendar.week",
  "four-week": "calendar.fourWeeks",
  month: "calendar.month",
  agenda: "calendar.agenda",
};

/** A weekday name; `weekday` is 0 = Sunday … 6 = Saturday, as the calendar numbers them. */
function weekdayName(locale: string, weekday: number, width: "long" | "short"): string {
  // 2023-01-01 was a Sunday.
  const date = new Date(Date.UTC(2023, 0, 1 + weekday));
  try {
    return new Intl.DateTimeFormat(locale, { weekday: width, timeZone: "UTC" }).format(date);
  } catch {
    return new Intl.DateTimeFormat("en", { weekday: width, timeZone: "UTC" }).format(date);
  }
}

function monthYear(locale: string, isoDate: string): string {
  const date = new Date(`${isoDate.slice(0, 7)}-01T00:00:00Z`);
  try {
    return new Intl.DateTimeFormat(locale, { month: "long", year: "numeric", timeZone: "UTC" }).format(date);
  } catch {
    return isoDate.slice(0, 7);
  }
}

export function calendarLabels(t: Translate, locale: string): CalendarViewLabels {
  const view = (v: CalendarView) => t(VIEW_KEY[v]);
  return {
    dayView: t("calendar.day"),
    weekView: t("calendar.week"),
    fourWeekView: t("calendar.fourWeeks"),
    monthView: t("calendar.month"),
    agendaView: t("calendar.agenda"),
    agendaEmpty: t("calendar.agendaEmpty"),
    previous: (v) => t("calendar.previous", { view: view(v) }),
    next: (v) => t("calendar.next", { view: view(v) }),
    today: t("calendar.today"),
    weekday: (wd) => weekdayName(locale, wd, "long"),
    weekdayAbbr: (wd) => weekdayName(locale, wd, "short"),
    periodTitle: (v, start, end, anchorDate) =>
      v === "month" || v === "agenda" ? monthYear(locale, anchorDate) : v === "day" ? start : `${start} – ${end}`,
    search: t("calendar.search"),
    export: t("calendar.export"),
    exportCsv: t("calendar.exportCsv"),
    exportExcel: t("calendar.exportExcel"),
    copy: t("calendar.copy"),
  };
}

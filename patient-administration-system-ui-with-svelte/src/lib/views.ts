// Pure mappings from PAS API shapes to the Lily components' inputs. No DOM,
// no Svelte, no clock, so they are unit-tested.

import type { CalendarEvent } from "@lilydesignsystem/svelte-calendar";
import type { GanttTask } from "@lilydesignsystem/svelte-gantt-chart";
import type { KanbanCard, KanbanColumn } from "@lilydesignsystem/svelte-kanban-board";

export interface AppointmentLike {
  id: string;
  person_id: string;
  start_datetime: string;
  end_datetime: string;
  status: string;
  reason?: string | null;
}

/** `YYYY-MM-DD` of an instant, in UTC. The PAS stores UTC. */
export const dayOf = (iso: string): string => iso.slice(0, 10);

/** `HH:MM` of an instant, in UTC. */
export const timeOf = (iso: string): string => iso.slice(11, 16);

/** Translates a catalogue key; `t` from `i18n.svelte.ts`, or a stand-in in tests. */
export type Translate = (key: string, values?: Record<string, string | number>) => string;

/** Catalogue key for an appointment status. */
export const statusKey = (s: string): string => `status.${s}`;

/** The no-show support flag for one appointment, as the PAS sends it (CAP-12). */
export interface NoShowLike {
  appointment_id: string;
  /** Absent until the weights are calibrated on real outcomes: staff then see the band and reasons only. */
  probability_percent?: number;
  band: "low" | "medium" | "high";
  factors: { name: string; effect_points: number }[];
  sample_size: number;
}

/** A shape and a word for each band, so the flag never depends on colour alone (WCAG 1.4.1). */
const BAND_MARK = { low: "●", medium: "◆", high: "▲" } as const;

/**
 * The flag and the reasons for it, in words: `▲ High. Based on 4 past visits. Past visits; Lead time`; the percentage and point sizes appear only once calibrated.
 * It is support for the front desk; nothing here changes the appointment.
 */
export function noshowText(n: NoShowLike, t: Translate): string {
  const why = n.factors
    .filter((f) => Math.abs(f.effect_points) >= 0.05 && f.name !== "base_rate")
    .map((f) =>
      // The point sizes are as uncalibrated as the percentage, so they travel with it: no number, no sizes.
      n.probability_percent === undefined
        ? t(`noshow.factor.${f.name}`)
        : `${t(`noshow.factor.${f.name}`)} ${f.effect_points > 0 ? "+" : ""}${t("noshow.points", { points: f.effect_points })}`,
    );
  const head = `${BAND_MARK[n.band]} ${t(`noshow.band.${n.band}`)}`;
  return [n.probability_percent === undefined ? `${head}.` : `${head}, ${n.probability_percent}%.`, t("noshow.sample", { n: n.sample_size }) + ".", why.join("; ")]
    .filter(Boolean)
    .join(" ");
}

export function appointmentEvents(rows: AppointmentLike[], t: Translate, noshow: NoShowLike[] = []): CalendarEvent[] {
  const flags = new Map(noshow.map((n) => [n.appointment_id, n]));
  return rows.map((a) => {
    const flag = flags.get(a.id);
    return {
      id: a.id,
      date: dayOf(a.start_datetime),
      title: `${timeOf(a.start_datetime)} ${a.reason?.trim() || t("appointments.generic")}`,
      done: a.status === "fulfilled",
      values: {
        time: `${timeOf(a.start_datetime)}–${timeOf(a.end_datetime)}`,
        status: t(statusKey(a.status)),
        ...(flag ? { support: noshowText(flag, t) } : {}),
      },
    };
  });
}

/** One parent task per day, with each appointment as a one-day milestone beneath it. */
export function appointmentGantt(rows: AppointmentLike[], t: Translate): GanttTask[] {
  const days = [...new Set(rows.map((a) => dayOf(a.start_datetime)))].sort();
  const tasks: GanttTask[] = days.map((d) => ({ id: `day:${d}`, label: d, start: d, end: d }));
  for (const a of [...rows].sort((x, y) => x.start_datetime.localeCompare(y.start_datetime))) {
    const d = dayOf(a.start_datetime);
    tasks.push({
      id: a.id,
      label: `${timeOf(a.start_datetime)} ${a.reason?.trim() || t("appointments.generic")}`,
      start: d,
      end: d,
      parentId: `day:${d}`,
      percentComplete: a.status === "fulfilled" ? 100 : 0,
    });
  }
  return tasks;
}

/** The inclusive `{start,end}` day range that covers the rows, padded to at least a week. */
export function ganttRange(rows: AppointmentLike[], today: string): { start: string; end: string } {
  const days = rows.map((a) => dayOf(a.start_datetime)).sort();
  const start = days[0] && days[0] < today ? days[0] : today;
  const last = days[days.length - 1] ?? today;
  const minEnd = new Date(Date.parse(`${start}T00:00:00Z`) + 6 * 86_400_000).toISOString().slice(0, 10);
  return { start, end: last > minEnd ? last : minEnd };
}

export interface WaitlistLike {
  id: string;
  person_id: string;
  target_service: string;
  priority: string;
  status: string;
}

const WAITLIST_COLUMN_IDS = ["waiting", "booked", "treated", "removed"] as const;

export const waitlistColumns = (t: Translate): KanbanColumn[] =>
  WAITLIST_COLUMN_IDS.map((id) => ({ id, title: t(`waitlist.column.${id}`) }));

const PRIORITY_RANK: Record<string, number> = { emergency: 0, two_week_wait: 1, urgent: 2, routine: 3 };

/** Catalogue key for a waitlist priority. */
export const priorityKey = (p: string): string => `priority.${p}`;

export function waitlistCards(rows: WaitlistLike[], t: Translate): KanbanCard[] {
  return [...rows]
    .sort((a, b) => (PRIORITY_RANK[a.priority] ?? 9) - (PRIORITY_RANK[b.priority] ?? 9))
    .map((w) => ({
      id: w.id,
      columnId: (WAITLIST_COLUMN_IDS as readonly string[]).includes(w.status) ? w.status : "waiting",
      // Subtitle and badges are separate text on the card. Since kanban 0.8.1 the board names the
      // cell by all of it, so a screen-reader user hears what a sighted one sees (WCAG 2.5.3).
      // (0.8.0 named it by the title alone, which forced everything into the title.)
      title: w.target_service,
      subtitle: t("waitlist.patient", { id: w.person_id.slice(0, 8) }),
      badges: [t(priorityKey(w.priority))],
    }));
}

export interface PersonLike {
  id?: string;
  name?: unknown;
  birth_date?: string;
  date_of_birth?: string;
  [k: string]: unknown;
}

/** Person names arrive as a string or as `{given, family}` (possibly arrays). */
export function personName(p: PersonLike): string {
  const n = p.name;
  if (typeof n === "string") return n;
  if (Array.isArray(n)) return personName({ name: n[0] });
  if (n && typeof n === "object") {
    const o = n as { given?: unknown; family?: unknown };
    const given = Array.isArray(o.given) ? o.given.join(" ") : (o.given ?? "");
    return `${given} ${o.family ?? ""}`.trim();
  }
  return "";
}

export function patientRows(rows: PersonLike[]): Record<string, unknown>[] {
  return rows.map((p) => ({
    id: p.id ?? "",
    name: personName(p),
    birthDate: p.birth_date ?? p.date_of_birth ?? "",
  }));
}

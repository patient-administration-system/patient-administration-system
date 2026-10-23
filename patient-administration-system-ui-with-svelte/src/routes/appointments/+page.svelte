<script lang="ts">
  import CalendarView from "@lilydesignsystem/svelte-calendar";
  import type { CalendarEventField, CalendarView as Mode } from "@lilydesignsystem/svelte-calendar";
  import { onMount } from "svelte";
  import type { PageData } from "./$types";
  import { calendarLabels } from "#lib/calendar-labels.js";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { appointmentEvents } from "#lib/views.js";

  let { data }: { data: PageData } = $props();

  let view = $state<Mode>("month");
  let anchorDate = $state("2000-01-01");
  let today = $state<string | undefined>(undefined);
  let search = $state("");
  onMount(() => {
    today = anchorDate = new Date().toISOString().slice(0, 10);
  });

  const events = $derived(appointmentEvents(data.appointments, t, data.noshow, data.adjustments));
  const fields = $derived<CalendarEventField[]>([
    { id: "time", header: t("col.time"), type: "text" },
    { id: "status", header: t("col.status"), type: "text" },
    // Only when the score is on and an appointment has one (CAP-12): the flag and why, beside the appointment.
    ...(data.adjustments.length > 0
      ? [{ id: "adjustments", header: t("adjustments.col"), type: "text" } as CalendarEventField]
      : []),
    ...(data.noshow.length > 0 ? [{ id: "support", header: t("noshow.col"), type: "text" } as CalendarEventField] : []),
  ]);
  const labels = $derived(calendarLabels(t, i18n.locale));
</script>

<svelte:head><title>{t("appointments.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("appointments.title")}</h1>
<CalendarView
  label={t("appointments.calendarLabel")}
  bind:view
  bind:anchorDate
  bind:search
  {events}
  {today}
  locale={i18n.locale}
  {labels}
  eventFields={fields}
  exportFilename="appointments"
/>

<script lang="ts">
  import CalendarView from "@lilydesignsystem/svelte-calendar";
  import type { CalendarEventField } from "@lilydesignsystem/svelte-calendar";
  import { onMount } from "svelte";
  import type { PageData } from "./$types";
  import { calendarLabels } from "#lib/calendar-labels.js";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { appointmentEvents, dayOf, statusKey, timeOf } from "#lib/views.js";

  let { data }: { data: PageData } = $props();

  let anchorDate = $state("2000-01-01");
  let today = $state<string | undefined>(undefined);
  onMount(() => {
    today = anchorDate = new Date().toISOString().slice(0, 10);
  });

  const events = $derived(appointmentEvents(data.appointments, t));
  const fields = $derived<CalendarEventField[]>([
    { id: "time", header: t("col.time"), type: "text" },
    { id: "status", header: t("col.status"), type: "text" },
  ]);
  const labels = $derived(calendarLabels(t, i18n.locale));
</script>

<svelte:head>
  <title>{t("myAppointments.title")} – {t("app.name")}</title>
  <link rel="stylesheet" href="/assets/themes/patient.css" />
</svelte:head>

<h1>{t("myAppointments.title")}</h1>
{#if data.appointments.length === 0}
  <p>{t("myAppointments.empty")}</p>
{:else}
  <CalendarView
    label={t("myAppointments.calendarLabel")}
    view="agenda"
    bind:anchorDate
    {events}
    {today}
    locale={i18n.locale}
    {labels}
    eventFields={fields}
  />
  <h2>{t("myAppointments.listHeading")}</h2>
  <table class="plain">
    <thead>
      <tr>
        <th>{t("col.date")}</th><th>{t("col.time")}</th><th>{t("col.reason")}</th><th>{t("col.status")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.appointments as a (a.id)}
        <tr>
          <td>{dayOf(a.start_datetime)}</td>
          <td>{timeOf(a.start_datetime)}–{timeOf(a.end_datetime)}</td>
          <td>{a.reason ?? t("appointments.generic")}</td>
          <td>{t(statusKey(a.status))}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<script lang="ts">
  import CalendarView from "@lilydesignsystem/svelte-calendar";
  import type { CalendarEventField } from "@lilydesignsystem/svelte-calendar";
  import { untrack } from "svelte";
  import type { PageData } from "./$types";
  import { calendarLabels } from "#lib/calendar-labels.js";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { appointmentEvents, dayOf, statusKey, timeOf } from "#lib/views.js";

  let { data }: { data: PageData } = $props();

  // From the server, so the first HTML shows this month, with or without JavaScript.
  let anchorDate = $state(untrack(() => data.today));

  const all = $derived([...data.upcoming, ...data.past]);
  const events = $derived(appointmentEvents(all, t, [], [], data.timeZone));
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
{#snippet rows(list: typeof data.upcoming)}
  <table class="plain">
    <thead>
      <tr>
        <th>{t("col.date")}</th><th>{t("col.time")}</th><th>{t("col.reason")}</th><th>{t("col.status")}</th>
      </tr>
    </thead>
    <tbody>
      {#each list as a (a.id)}
        <tr>
          <td>{dayOf(a.start_datetime, data.timeZone)}</td>
          <td>{timeOf(a.start_datetime, data.timeZone)}–{timeOf(a.end_datetime, data.timeZone)}</td>
          <td>{a.reason ?? t("appointments.generic")}</td>
          <td>{t(statusKey(a.status))}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/snippet}

{#if all.length === 0}
  <p>{t("myAppointments.empty")}</p>
{:else}
  <p>{t("time.zoneNote", { zone: data.timeZone })}</p>
  <CalendarView
    label={t("myAppointments.calendarLabel")}
    view="agenda"
    bind:anchorDate
    {events}
    today={data.today}
    locale={i18n.locale}
    {labels}
    eventFields={fields}
  />
  <h2>{t("myAppointments.upcoming")}</h2>
  {#if data.upcoming.length === 0}
    <p>{t("myAppointments.noUpcoming")}</p>
  {:else}
    {@render rows(data.upcoming)}
  {/if}
  {#if data.past.length > 0}
    <h2>{t("myAppointments.past")}</h2>
    {@render rows(data.past)}
  {/if}
  {#if data.olderCursor}
    <p><a href="?older={encodeURIComponent(data.olderCursor)}">{t("myAppointments.older")}</a></p>
  {/if}
{/if}

<script lang="ts">
  import CalendarView from "@lilydesignsystem/svelte-calendar";
  import type { CalendarEventField } from "@lilydesignsystem/svelte-calendar";
  import { onMount } from "svelte";
  import type { PageData } from "./$types";
  import { appointmentEvents, dayOf, statusLabel, timeOf } from "#lib/views.js";

  let { data }: { data: PageData } = $props();

  let anchorDate = $state("2000-01-01");
  let today = $state<string | undefined>(undefined);
  onMount(() => {
    today = anchorDate = new Date().toISOString().slice(0, 10);
  });

  const events = $derived(appointmentEvents(data.appointments));
  const fields: CalendarEventField[] = [
    { id: "time", header: "Time", type: "text" },
    { id: "status", header: "Status", type: "text" },
  ];
</script>

<svelte:head>
  <title>My appointments – Patient Administration System</title>
  <link rel="stylesheet" href="/assets/themes/patient.css" />
</svelte:head>

<h1>My appointments</h1>
{#if data.appointments.length === 0}
  <p>You have no appointments.</p>
{:else}
  <CalendarView label="My appointments" view="agenda" bind:anchorDate {events} {today} eventFields={fields} />
  <h2>As a list</h2>
  <table class="plain">
    <thead><tr><th>Date</th><th>Time</th><th>Reason</th><th>Status</th></tr></thead>
    <tbody>
      {#each data.appointments as a (a.id)}
        <tr>
          <td>{dayOf(a.start_datetime)}</td>
          <td>{timeOf(a.start_datetime)}–{timeOf(a.end_datetime)}</td>
          <td>{a.reason ?? "Appointment"}</td>
          <td>{statusLabel(a.status)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<script lang="ts">
  import GanttChart from "@lilydesignsystem/svelte-gantt-chart";
  import type { PageData } from "./$types";
  import { appointmentGantt, ganttRange } from "#lib/views.js";

  let { data }: { data: PageData } = $props();
  const tasks = $derived(appointmentGantt(data.appointments));
  const range = $derived(ganttRange(data.appointments, data.today));
</script>

<svelte:head><title>Schedule – Patient Administration System</title></svelte:head>
<h1>Schedule: next four weeks</h1>
{#if tasks.length === 0}
  <p>No appointments are booked in the next four weeks.</p>
{:else}
  <GanttChart label="Appointment schedule" {tasks} {range} today={data.today} timeUnit="day" readonly />
{/if}

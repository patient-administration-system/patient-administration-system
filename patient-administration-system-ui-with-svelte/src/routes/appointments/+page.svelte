<script lang="ts">
  import CalendarView from "@lilydesignsystem/svelte-calendar";
  import type { CalendarEventField, CalendarView as Mode } from "@lilydesignsystem/svelte-calendar";
  import { onMount } from "svelte";
  import type { PageData } from "./$types";
  import { appointmentEvents } from "#lib/views.js";

  let { data }: { data: PageData } = $props();

  let view = $state<Mode>("month");
  let anchorDate = $state("2000-01-01");
  let today = $state<string | undefined>(undefined);
  let search = $state("");
  onMount(() => {
    today = anchorDate = new Date().toISOString().slice(0, 10);
  });

  const events = $derived(appointmentEvents(data.appointments));
  const fields: CalendarEventField[] = [
    { id: "time", header: "Time", type: "text" },
    { id: "status", header: "Status", type: "text" },
  ];
</script>

<svelte:head><title>Appointments – Patient Administration System</title></svelte:head>
<h1>Appointments</h1>
<CalendarView label="Appointments" bind:view bind:anchorDate bind:search {events} {today} eventFields={fields} exportFilename="appointments" />

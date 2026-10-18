<script lang="ts">
  import KanbanBoard from "@lilydesignsystem/svelte-kanban-board";
  import type { PageData } from "./$types";
  import { WAITLIST_COLUMNS, waitlistCards } from "#lib/views.js";

  let { data }: { data: PageData } = $props();
  const cards = $derived(waitlistCards(data.entries));
</script>

<svelte:head><title>Waitlist – Patient Administration System</title></svelte:head>
<h1>Waitlist</h1>
<form method="GET">
  <label for="service">Service</label>
  <input id="service" name="service" type="text" value={data.service} />
  <button class="button" type="submit">Show</button>
</form>
{#if data.entries.length === 0}
  <p>Nobody is waiting for {data.service}.</p>
{:else}
  <!-- Read-only: status changes go through the booking and removal actions, not drag and drop. -->
  <KanbanBoard label={`Waitlist for ${data.service}`} columns={WAITLIST_COLUMNS} {cards} readonly />
{/if}

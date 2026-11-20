<script lang="ts">
  import KanbanBoard from "@lilydesignsystem/svelte-kanban-board";
  import type { PageData } from "./$types";
  import { t } from "#lib/i18n.svelte.js";
  import { waitlistCards, waitlistColumns } from "#lib/views.js";

  let { data }: { data: PageData } = $props();
  const cards = $derived(waitlistCards(data.entries, t));
  const columns = $derived(waitlistColumns(t));
</script>

<svelte:head><title>{t("waitlist.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("waitlist.title")}</h1>
<form method="GET">
  <label for="service">{t("waitlist.service")}</label>
  <input id="service" name="service" type="text" value={data.service} />
  <button class="button" type="submit">{t("waitlist.show")}</button>
</form>
{#if data.entries.length === 0}
  <p>{t("waitlist.nobody", { service: data.service })}</p>
{:else}
  <!-- Read-only: status changes go through the booking and removal actions, not drag and drop. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (the overflowing region must be keyboard-scrollable) -->
  <div class="board-scroll" role="region" tabindex="0" aria-label={t("waitlist.boardLabel", { service: data.service })}>
    <KanbanBoard label={t("waitlist.boardLabel", { service: data.service })} {columns} {cards} readonly />
  </div>
{/if}

<style>
  .board-scroll {
    max-width: 100%;
    overflow-x: auto;
  }
</style>

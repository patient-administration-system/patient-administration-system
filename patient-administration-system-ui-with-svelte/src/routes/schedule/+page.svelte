<script lang="ts">
  import GanttChart from "@lilydesignsystem/svelte-gantt-chart";
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { appointmentGantt, ganttRange } from "#lib/views.js";

  let { data }: { data: PageData } = $props();
  const tasks = $derived(appointmentGantt(data.appointments, t));
  const range = $derived(ganttRange(data.appointments, data.today));
</script>

<svelte:head><title>{t("schedule.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("schedule.heading")}</h1>
{#if tasks.length === 0}
  <p>{t("schedule.empty")}</p>
{:else}
  <GanttChart label={t("schedule.chartLabel")} {tasks} {range} today={data.today} timeUnit="day" locale={i18n.locale} readonly />
{/if}

<script lang="ts">
  import DataGrid from "@lilydesignsystem/svelte-data-grid";
  import type { DataGridColumn } from "@lilydesignsystem/svelte-data-grid";
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";

  let { data }: { data: PageData } = $props();
  const columns = $derived<DataGridColumn[]>([
    { id: "name", header: t("col.name"), sortable: true },
    { id: "birthDate", header: t("col.birthDate"), sortable: true },
    { id: "id", header: t("col.patientId") },
  ]);
</script>

<svelte:head><title>{t("patients.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("patients.title")}</h1>
<form method="GET" role="search">
  <label for="q">{t("patients.search")}</label>
  <input id="q" name="q" type="text" value={data.q} />
  <button class="button" type="submit">{t("patients.searchButton")}</button>
</form>
{#if data.q}
  {#if data.rows.length === 0}
    <p>{t("patients.none", { q: data.q })}</p>
  {:else}
    <DataGrid label={t("patients.gridLabel")} {columns} rows={data.rows} rowId={(r) => String(r.id)} locale={i18n.locale} exportFilename="patients" />
  {/if}
{/if}

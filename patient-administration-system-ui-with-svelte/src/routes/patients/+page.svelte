<script lang="ts">
  import DataGrid from "@lilydesignsystem/svelte-data-grid";
  import type { DataGridColumn } from "@lilydesignsystem/svelte-data-grid";
  import type { PageData } from "./$types";

  let { data }: { data: PageData } = $props();
  const columns: DataGridColumn[] = [
    { id: "name", header: "Name", sortable: true },
    { id: "birthDate", header: "Date of birth", sortable: true },
    { id: "id", header: "Patient id" },
  ];
</script>

<svelte:head><title>Patients – Patient Administration System</title></svelte:head>
<h1>Patients</h1>
<form method="GET" role="search">
  <label for="q">Search by name or identifier</label>
  <input id="q" name="q" type="text" value={data.q} />
  <button class="button" type="submit">Search</button>
</form>
{#if data.q}
  {#if data.rows.length === 0}
    <p>No patients match “{data.q}”.</p>
  {:else}
    <DataGrid label="Patients" {columns} rows={data.rows} rowId={(r) => String(r.id)} exportFilename="patients" />
  {/if}
{/if}

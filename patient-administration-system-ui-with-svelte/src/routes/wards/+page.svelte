<script lang="ts">
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { localizePath } from "#lib/locales.js";

  let { data }: { data: PageData } = $props();
  const when = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
</script>

<svelte:head><title>{t("ward.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("ward.title")}</h1>
<p>{t("ward.intro")}</p>
<form method="GET" role="search">
  <label for="ward">{t("ward.id")}</label>
  <input id="ward" name="ward" type="text" value={data.ward} />
  <button class="button" type="submit">{t("waitlist.show")}</button>
</form>
{#if !data.ward}
  <h2>{t("ward.choose")}</h2>
  {#if data.places.every((p) => p.wards.length === 0)}
    <p>{t("ward.none")}</p>
  {:else}
    {#each data.places.filter((p) => p.wards.length > 0) as p (p.facility.id)}
      <h3>{p.facility.name}</h3>
      <ul>
        {#each p.wards as w (w.id)}
          <li><a href={localizePath(`/wards?ward=${w.id}`, i18n.locale)}>{w.name}</a></li>
        {/each}
      </ul>
    {/each}
  {/if}
{:else if data.invalid}
  <p role="alert">{t("history.invalid")}</p>
{:else if data.overview}
  {@const o = data.overview}
  <p>{t("ward.summary", { occupied: o.occupancy.occupied, total: o.occupancy.total_beds })}</p>
  <table>
    <caption>{t("ward.caption")}</caption>
    <thead>
      <tr>
        <th scope="col">{t("ward.col.room")}</th>
        <th scope="col">{t("ward.col.bed")}</th>
        <th scope="col">{t("col.status")}</th>
        <th scope="col">{t("col.patientId")}</th>
        <th scope="col">{t("ward.col.since")}</th>
        <th scope="col">{t("ward.col.days")}</th>
      </tr>
    </thead>
    <tbody>
      {#each o.beds as b (b.bed_id)}
        <tr>
          <td>{b.room}</td>
          <td>{b.bed}</td>
          <td>{t(`bed.${b.status}`)}</td>
          <td>{b.person_id ?? t("ward.empty")}</td>
          <td>{b.in_bed_since ? when(b.in_bed_since) : "–"}</td>
          <td>{b.days_in_hospital ?? "–"}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

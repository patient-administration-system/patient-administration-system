<script lang="ts">
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import ListCut from "#lib/ListCut.svelte";

  let { data }: { data: PageData } = $props();
  const day = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeZone: "UTC" }).format(new Date(iso));
  const MARK: Record<string, string> = { past: "✕", near: "!", within: "✓", no_target: "–" };
</script>

<svelte:head><title>{t("suspicions.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("suspicions.title")}</h1>
<p>{t("suspicions.intro")}</p>
<form method="GET">
  <label for="condition">{t("suspicions.condition")}</label>
  <select id="condition" name="condition">
    <option value="" selected={data.condition === null}>{t("suspicions.all")}</option>
    {#each data.conditions as c (c)}
      <option value={c} selected={data.condition === c}>{t(`condition.${c}`)}</option>
    {/each}
  </select>
  <button class="button" type="submit">{t("waitlist.show")}</button>
</form>
{#if data.rows.length === 0}
  <p>{t("suspicions.none")}</p>
{:else}
  <ListCut at={data.cutAt} next={data.nextHref} first={data.firstHref} />
  <table>
    <caption>{t("suspicions.caption")}</caption>
    <thead>
      <tr>
        <th scope="col">{t("col.patientId")}</th>
        <th scope="col">{t("suspicions.condition")}</th>
        <th scope="col">{t("suspicions.col.site")}</th>
        <th scope="col">{t("suspicions.col.since")}</th>
        <th scope="col">{t("suspicions.col.days")}</th>
        <th scope="col">{t("suspicions.col.target")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.rows as s (s.id)}
        <tr>
          <td>{s.person_id}</td>
          <td>{t(`condition.${s.condition}`)}</td>
          <td>{s.tumour_site ? t(`tumour.${s.tumour_site}`) : "–"}</td>
          <td>{day(s.suspected_on)}</td>
          <td>{s.days_waited}</td>
          <td>
            <span aria-hidden="true">{MARK[s.target_status]}</span>
            {t(`target.${s.target_status}`, { days: s.target_days ?? 0 })}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<script lang="ts">
  import type { ActionData, PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();
  const day = (iso: string) => new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium" }).format(new Date(iso));
</script>

<svelte:head><title>{t("waits.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("waits.title")}</h1>
<p>{t("waits.intro")}</p>
{#if form?.asked}
  <p role="status">{t("waits.asked")}</p>
{:else if form?.messageKey}
  <p role="alert">{t(form.messageKey)}</p>
{/if}
{#if data.due.length === 0}
  <p>{t("waits.none")}</p>
{:else}
  <table>
    <caption>{t("waits.caption")}</caption>
    <thead>
      <tr>
        <th scope="col">{t("waits.col.patient")}</th>
        <th scope="col">{t("waits.col.service")}</th>
        <th scope="col">{t("waits.col.weeks")}</th>
        <th scope="col">{t("waits.col.last")}</th>
        <th scope="col">{t("waits.ask")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.due as d (d.pathway_id)}
        <tr>
          <td>{d.person_id}</td>
          <td>{d.target_service}</td>
          <td>{d.weeks_waited}</td>
          <td>{d.last_check_at ? day(d.last_check_at) : t("waits.never")}</td>
          <td>
            <form method="POST" action="?/ask">
              <input type="hidden" name="pathway" value={d.pathway_id} />
              <button class="button" type="submit">{t("waits.ask")}</button>
            </form>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

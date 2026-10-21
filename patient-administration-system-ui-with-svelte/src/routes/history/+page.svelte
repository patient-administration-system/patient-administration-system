<script lang="ts">
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";

  let { data }: { data: PageData } = $props();
  const when = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
  const what = (r: { kind: string | null; action: string | null }) => [r.kind, r.action].filter(Boolean).join(": ");
</script>

<svelte:head><title>{t("history.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("history.title")}</h1>
<form method="GET" role="search">
  <label for="appointment">{t("history.appointment")}</label>
  <input id="appointment" name="appointment" type="text" value={data.appointment} autocomplete="off" />
  <button class="button" type="submit">{t("history.show")}</button>
</form>
<form method="GET" role="search">
  <label for="patient">{t("history.patient")}</label>
  <input id="patient" name="patient" type="text" value={data.patient} autocomplete="off" />
  <button class="button" type="submit">{t("history.show")}</button>
</form>
{#if data.invalid}
  <p role="alert">{t("history.invalid")}</p>
{:else if data.rows}
  {#if data.rows.length === 0}
    <p>{t("history.none")}</p>
  {:else}
    <table>
      <caption>{data.kind === "appointment" ? t("history.captionAppointment") : t("history.captionContact")}</caption>
      <thead>
        <tr>
          <th scope="col">{t("history.col.when")}</th>
          <th scope="col">{t("history.col.what")}</th>
          <th scope="col">{t("history.col.by")}</th>
          <th scope="col">{t("history.col.why")}</th>
        </tr>
      </thead>
      <tbody>
        {#each data.rows as r, i (i)}
          <tr>
            <td>{when(r.at)}</td>
            <td>{what(r)}</td>
            <td>{r.by ?? ""}</td>
            <td>{r.reason_code ?? ""}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
{/if}

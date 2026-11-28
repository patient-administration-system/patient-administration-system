<script lang="ts">
  import type { PageData } from "./$types";
  import { t } from "#lib/i18n.svelte.js";
  import { localizePath } from "#lib/locales.js";
  import { i18n } from "#lib/i18n.svelte.js";

  let { data }: { data: PageData } = $props();
  const when = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
</script>

<svelte:head><title>{t("home.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("home.title")}</h1>
{#if data.rides.length > 0}
  <div class="alert" role="alert" data-testid="transport-alert">
    <p>
      <strong>
        {data.ridesMore
          ? t("home.transportCountMore", { count: data.rides.length })
          : t("home.transportCount", { count: data.rides.length })}
      </strong>
    </p>
    <ul>
      {#each data.rides as r (r.booking_id)}
        <li data-booking={r.booking_id}>
          {when(r.appointment_start)}: {t(`home.transportStatus.${r.status}`)}
          <a href={localizePath(`/enquiry?patient=${encodeURIComponent(r.person_id)}`, i18n.locale)}>
            {t("home.transportPatient")}
          </a>
        </li>
      {/each}
    </ul>
  </div>
{/if}
<p>{t("home.intro")}</p>
<ul class="tiles" aria-label={t("home.label")}>
  {#each data.tiles as tile (tile.href)}
    <li>
      <a href={localizePath(tile.href, i18n.locale)}>{t(tile.key)}</a>
      {#if tile.href === "/suspicions" && data.past > 0}
        <span>
          {data.pastIsMore
            ? t("home.pastTargetMore", { count: data.past })
            : t("home.pastTarget", { count: data.past })}
        </span>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .alert {
    margin-block: 1rem;
    padding: 1rem;
    border: 2px solid currentColor;
    border-radius: 0.5rem;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: 1rem;
    padding: 0;
    list-style: none;
  }
  .tiles li {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 1rem;
    border: 1px solid currentColor;
    border-radius: 0.5rem;
  }
</style>

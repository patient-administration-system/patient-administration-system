<script lang="ts">
  import type { PageData } from "./$types";
  import { t } from "#lib/i18n.svelte.js";
  import { localizePath } from "#lib/locales.js";
  import { i18n } from "#lib/i18n.svelte.js";

  let { data }: { data: PageData } = $props();
</script>

<svelte:head><title>{t("home.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("home.title")}</h1>
<p>{t("home.intro")}</p>
<ul class="tiles" aria-label={t("home.label")}>
  {#each data.tiles as tile (tile.href)}
    <li>
      <a href={localizePath(tile.href, i18n.locale)}>{t(tile.key)}</a>
      {#if tile.href === "/suspicions" && data.past > 0}
        <span
          >{data.pastIsMore
            ? t("home.pastTargetMore", { count: data.past })
            : t("home.pastTarget", { count: data.past })}</span
        >
      {/if}
    </li>
  {/each}
</ul>

<style>
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

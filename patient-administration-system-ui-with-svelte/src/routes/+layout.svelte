<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import type { Snippet } from "svelte";
  import type { LayoutData } from "./$types";
  import { LOCALES, LOCALE_LABELS, i18n, localeHref, t } from "#lib/i18n.svelte.js";
  import { localizePath, splitPath } from "#lib/locales.js";
  import ThemePicker from "@lilydesignsystem/svelte-theme-picker";
  import { SCHEMES, THEME_COOKIE, THEME_STORAGE_KEY } from "#lib/theme.js";

  let { data, children }: { data: LayoutData; children: Snippet } = $props();

  // The URL decides the language. Set it during server rendering (this runs before the page
  // renders, synchronously) and again whenever navigation changes it.
  // svelte-ignore state_referenced_locally (the initial value is exactly what server rendering needs)
  i18n.set(data.locale ?? "en-001");
  $effect.pre(() => i18n.set(data.locale ?? "en-001"));

  const rest = $derived(splitPath(page.url.pathname).rest + page.url.search);

  // The server read the choice from the picker's cookie, so the picker starts where the first paint is.
  // svelte-ignore state_referenced_locally (the picker owns the value after the first render)
  let theme = $state<string>(data.theme);
</script>

<a class="skip-link" href="#main">{t("app.skip")}</a>
<header class="site">
  <a class="brand" href={localeHref("/")}>{t("app.name")}</a>
  <nav aria-label={t("app.mainNav")}>
    {#each data.nav as item (item.href)}
      <a href={localeHref(item.href)}>{t(item.key)}</a>
    {/each}
  </nav>
  <details class="languages">
    <summary>{t("app.language")}</summary>
    <ul>
      {#each LOCALES as code (code)}
        <li>
          <a
            href={localizePath(rest, code)}
            lang={code}
            hreflang={code}
            aria-current={code === i18n.locale ? "true" : undefined}>{LOCALE_LABELS[code]}</a
          >
        </li>
      {/each}
    </ul>
  </details>
  <!-- NFR-30: Follow this device, Light, Dark. The choice is kept in the `theme` cookie and in localStorage,
       only once a person chooses (see /privacy). -->
  <ThemePicker
    class="themes"
    label={t("app.theme.menu")}
    themesUrl="/assets/themes/scheme/"
    themes={[...SCHEMES]}
    systemLabel={t("app.theme.device")}
    themeLabels={{ light: t("app.theme.light"), dark: t("app.theme.dark") }}
    cookieName={THEME_COOKIE}
    storageKey={THEME_STORAGE_KEY}
    bind:value={theme}
  >
    {#snippet children({ open }: { open: boolean })}
      <span>{t("app.theme.menu")}</span>
      <span aria-hidden="true">{open ? "▴" : "▾"}</span>
    {/snippet}
  </ThemePicker>
  {#if data.userName}
    <span>{data.userName}</span>
    <form method="POST" action="/signout"><button type="submit">{t("app.signOut")}</button></form>
  {:else}
    <a href={localeHref("/signin")}>{t("app.signIn")}</a>
  {/if}
</header>
<main id="main">{@render children()}</main>
<footer class="site">
  <a href={localeHref("/privacy")}>{t("app.privacy")}</a>
</footer>

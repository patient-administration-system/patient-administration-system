<script lang="ts">
  import { page } from "$app/state";
  import { localeHref, t } from "#lib/i18n.svelte.js";

  // Show a translated message for the statuses we raise ourselves; the server's own English text is
  // not shown, so the page stays in the reader's language.
  const known = [401, 403, 404, 502];
  const message = $derived(known.includes(page.status) ? t(`error.${page.status}`) : t("error.generic"));
</script>

<svelte:head><title>{t("error.title", { status: page.status })} – {t("app.name")}</title></svelte:head>
<h1>{t("error.title", { status: page.status })}</h1>
<p class="error">{message}</p>
<p><a href={localeHref("/")}>{t("error.back")}</a></p>

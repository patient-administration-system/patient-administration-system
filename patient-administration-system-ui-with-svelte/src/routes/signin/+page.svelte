<script lang="ts">
  import type { PageData } from "./$types";
  import { t } from "#lib/i18n.svelte.js";
  let { data }: { data: PageData } = $props();
</script>

<svelte:head><title>{t("signin.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("signin.title")}</h1>
{#if data.errorKey}<p class="error" role="alert">{t(data.errorKey)}</p>{/if}
{#if data.enabled}
  <p>{t("signin.intro")}</p>
  <a class="button" href={`/signin/oidc?return=${encodeURIComponent(data.returnTo)}`}>{t("app.signIn")}</a>
{:else}
  <p class="error">{t("signin.notConfigured")}</p>
{/if}

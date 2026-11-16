<script lang="ts">
  import { t } from "#lib/i18n.svelte.js";
</script>

<!-- What these pages keep in the browser, and why. Public: it must be readable before signing in. -->
<svelte:head><title>{t("privacy.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("privacy.title")}</h1>
<p>{t("privacy.intro")}</p>

<h2>{t("privacy.choices.heading")}</h2>
<p>{t("privacy.choices.body")}</p>
<ul>
  <li>{t("privacy.theme")}</li>
  <li>{t("privacy.language")}</li>
</ul>

<h2>{t("privacy.signin.heading")}</h2>
<ul>
  <li>{t("privacy.signin.session")}</li>
  <li>{t("privacy.signin.tx")}</li>
</ul>
<p>{t("privacy.none")}</p>

<h2>{t("privacy.remove.heading")}</h2>
<p>{t("privacy.remove.body")}</p>

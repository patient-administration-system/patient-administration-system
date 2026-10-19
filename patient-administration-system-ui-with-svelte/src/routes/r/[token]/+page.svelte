<script lang="ts">
  import { enhance } from "$app/forms";
  import { page } from "$app/state";
  import type { ActionData, PageData } from "./$types";
  import { t, i18n } from "#lib/i18n.svelte.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();

  // An action can discover the link is no longer usable (already used, expired): show that instead.
  const state = $derived(form && "state" in form && form.state ? form.state : data.state);
  const result = $derived(form && "result" in form ? form.result : null);

  // The time reads in the language of the page and the clinic's time zone, as a patient expects it.
  const when = $derived(
    data.startsAt
      ? new Intl.DateTimeFormat(i18n.locale, { dateStyle: "full", timeStyle: "short", timeZone: data.timeZone }).format(
          new Date(data.startsAt),
        )
      : "",
  );
</script>

<svelte:head>
  <title>{t("link.title")} – {t("app.name")}</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<h1>{t("link.title")}</h1>

{#if result === "confirmed" || result === "cancelled" || result === "reschedule_requested" || result === "rejected"}
  <!-- What just happened wins over the link's new "used" state, which the reload after the tap reports. -->
  <p role="status">{t(`link.done.${result}`)}</p>
{:else if result === "help"}
  <p role="status">{t("link.done.help")}</p>
{:else if state !== "open"}
  <p role="status">{t(`link.${state}`)}</p>
{:else if result === "needs_confirmation"}
  <h2>{t("link.sure.title")}</h2>
  <p>{t("link.sure.body")}</p>
  <form method="POST" action="?/cancelSure" use:enhance>
    <button class="button" type="submit">{t("link.sure.yes")}</button>
  </form>
  <p><a href={page.url.pathname}>{t("link.sure.no")}</a></p>
{:else}
  {#if when}<p>{t("link.when", { when })}</p>{/if}
  <p>{t("link.question")}</p>
  <div class="actions">
    <form method="POST" action="?/confirm" use:enhance><button class="button" type="submit">{t("link.confirm")}</button></form>
    <form method="POST" action="?/reschedule" use:enhance><button class="button" type="submit">{t("link.reschedule")}</button></form>
    <form method="POST" action="?/cancel" use:enhance><button class="button" type="submit">{t("link.cancel")}</button></form>
    <form method="POST" action="?/help" use:enhance><button class="button" type="submit">{t("link.help")}</button></form>
  </div>
{/if}

<style>
  .actions {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 24rem;
  }
  .actions button {
    width: 100%;
  }
</style>

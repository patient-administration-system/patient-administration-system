<script lang="ts">
  import { enhance } from "$app/forms";
  import type { ActionData, PageData } from "./$types";
  import { t, i18n } from "#lib/i18n.svelte.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();

  // An action can discover the offer is no longer usable (already answered, expired): show that instead.
  const state = $derived(form && "state" in form && form.state ? form.state : data.state);
  const result = $derived(form && "result" in form ? form.result : null);

  // Times read in the language of the page and the clinic's time zone, as a patient expects them.
  const format = (iso: string, timeZone: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "full", timeStyle: "short", timeZone }).format(new Date(iso));
  const when = $derived(data.startsAt ? format(data.startsAt, data.timeZone) : "");
  const accepted = $derived(
    form && "startsAt" in form && form.startsAt ? format(form.startsAt, form.timeZone ?? data.timeZone) : "",
  );
</script>

<svelte:head>
  <title>{t("offer.title")} – {t("app.name")}</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<h1>{t("offer.title")}</h1>

{#if result === "accepted"}
  <p role="status">{t("offer.done.accepted", { when: accepted })}</p>
{:else if result === "declined"}
  <p role="status">{t("offer.done.declined")}</p>
{:else if result === "gone" || state === "gone"}
  <p role="status">{t("offer.done.gone")}</p>
{:else if state !== "open"}
  <p role="status">{t(`link.${state}`)}</p>
{:else}
  {#if when}<p>{t("offer.body", { when })}</p>{/if}
  <div class="actions">
    <form method="POST" action="?/accept" use:enhance>
      <button class="button" type="submit">{t("offer.accept")}</button>
    </form>
    <form method="POST" action="?/decline" use:enhance>
      <button class="button" type="submit">{t("offer.decline")}</button>
    </form>
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

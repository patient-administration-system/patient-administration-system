<script lang="ts">
  import { enhance } from "$app/forms";
  import { page } from "$app/state";
  import type { ActionData, PageData } from "./$types";
  import { t, i18n } from "#lib/i18n.svelte.js";
  import { REASONS } from "#lib/reasons.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();

  // An action can discover the link is no longer usable (already used, expired): show that instead.
  const state = $derived(form && "state" in form && form.state ? form.state : data.state);
  const result = $derived(form && "result" in form ? form.result : null);

  const signpost = $derived(form && "signpost" in form && form.signpost ? form.signpost : "");
  const retry = $derived(form && "retry" in form && form.retry === true);
  const options = $derived(form && "options" in form ? (form.options ?? []) : []);

  // Times read in the language of the page and the clinic's time zone, as a patient expects them.
  const format = (iso: string, timeZone: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "full", timeStyle: "short", timeZone }).format(new Date(iso));
  const when = $derived(data.startsAt ? format(data.startsAt, data.timeZone) : "");
  const movedTo = $derived(
    form && "startsAt" in form && form.startsAt ? format(form.startsAt, data.timeZone) : "",
  );
</script>

<svelte:head>
  <title>{t("link.title")} – {t("app.name")}</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<h1>{t("link.title")}</h1>

{#if result === "confirmed" || result === "cancelled" || result === "reschedule_requested" || result === "held_for_call" || result === "rejected"}
  <!-- What just happened wins over the link's new "used" state, which the reload after the tap reports. -->
  <p role="status">{t(`link.done.${result}`)}</p>
  {#if result === "cancelled" && signpost}<p>{signpost}</p>{/if}
{:else if result === "rebooked"}
  <p role="status">{t("link.done.rebooked", { when: movedTo })}</p>
{:else if result === "rebook_options" && options.length > 0}
  <h2>{t("link.rebook.title")}</h2>
  <p>{t("link.rebook.body")}</p>
  <ul class="times">
    {#each options as o (o.slot_id)}
      <li>
        <form method="POST" action="?/rebook" use:enhance>
          <input type="hidden" name="slot_id" value={o.slot_id} />
          <button class="button" type="submit">{format(o.starts_at, data.timeZone)}</button>
        </form>
      </li>
    {/each}
  </ul>
  <p><a href={page.url.pathname}>{t("link.rebook.keep")}</a></p>
{:else if result === "help"}
  <p role="status">{t("link.done.help")}</p>
{:else if state !== "open"}
  <p role="status">{t(`link.${state}`)}</p>
{:else if result === "needs_confirmation"}
  <h2>{t("link.sure.title")}</h2>
  <p>{t("link.sure.body")}</p>
  <form method="POST" action="?/cancelSure" use:enhance>
    <fieldset>
      <legend>{t("link.why.title")}</legend>
      {#each REASONS as r (r)}
        <label><input type="radio" name="barrier" value={r} /> {t(`link.why.${r}`)}</label>
      {/each}
    </fieldset>
    <button class="button" type="submit">{t("link.sure.yes")}</button>
  </form>
  <p><a href={page.url.pathname}>{t("link.sure.no")}</a></p>
{:else}
  {#if retry}<p role="status">{t("link.rebook.taken")}</p>{/if}
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
  .times {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 24rem;
  }
  .times button,
  .actions button {
    width: 100%;
  }
</style>

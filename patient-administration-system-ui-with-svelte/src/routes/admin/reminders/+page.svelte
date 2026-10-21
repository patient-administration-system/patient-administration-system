<script lang="ts">
  import { enhance } from "$app/forms";
  import type { ActionData, PageData } from "./$types";
  import { CHANNELS } from "#lib/channels.js";
  import { t } from "#lib/i18n.svelte.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();
</script>

<svelte:head><title>{t("admin.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("admin.title")}</h1>
<p>{t("admin.intro")}</p>
<p>
  {t("admin.variables")}
  <code>{"{{ patient.name.given }}"}</code>, <code>{"{{ appointment.when }}"}</code>,
  <code>{"{{ appointment.start }}"}</code>,
  <code>{"{{ appointment.reason }}"}</code>, <code>{"{{ portal_url }}"}</code>
</p>

{#each data.templates as t0 (t0.id)}
  <form method="POST" action="?/save" use:enhance>
    <fieldset>
      <legend>{t0.name} <span lang={t0.locale.split("-")[0]}>({t0.locale})</span></legend>
      {#if form?.id === t0.id && form?.messageKey}<p class="error" role="alert">{t(form.messageKey)}</p>{/if}
      {#if form?.id === t0.id && form?.saved}<p role="status">{t("admin.saved")}</p>{/if}
      <input type="hidden" name="id" value={t0.id} />
      <label for={`subject-${t0.id}`}>{t("admin.subject")}</label>
      <input id={`subject-${t0.id}`} name="subject" type="text" value={t0.subject} required />
      <label for={`body-${t0.id}`}>{t("admin.message")}</label>
      <textarea id={`body-${t0.id}`} name="body_tera" required>{t0.body_tera}</textarea>
      <fieldset>
        <legend>{t("admin.channels")}</legend>
        {#each CHANNELS as c (c.id)}
          <label>
            <input type="checkbox" name="channels" value={c.id} checked={t0.channels.includes(c.id)} />
            {t(`channel.${c.id}`)}
          </label>
        {/each}
      </fieldset>
      <label><input type="checkbox" name="active" checked={t0.active} /> {t("admin.active")}</label>
      <p><button class="button" type="submit">{t("admin.save")}</button></p>
    </fieldset>
  </form>
{:else}
  <p>{t("admin.none")}</p>
{/each}

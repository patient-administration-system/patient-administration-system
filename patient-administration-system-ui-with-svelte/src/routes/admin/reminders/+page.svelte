<script lang="ts">
  import { enhance } from "$app/forms";
  import type { ActionData, PageData } from "./$types";
  import { CHANNELS } from "#lib/channels.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();
</script>

<svelte:head><title>Reminder templates – Patient Administration System</title></svelte:head>
<h1>Reminder templates</h1>
<p>
  The message each patient receives. Channels are tried in the order listed until one reaches the patient.
  You can use <code>{"{{ patient.name.given }}"}</code>, <code>{"{{ appointment.start }}"}</code>,
  <code>{"{{ appointment.reason }}"}</code>.
</p>

{#each data.templates as t (t.id)}
  <form method="POST" action="?/save" use:enhance>
    <fieldset>
      <legend>{t.name}</legend>
      {#if form?.id === t.id && form?.message}<p class="error" role="alert">{form.message}</p>{/if}
      {#if form?.id === t.id && form?.saved}<p role="status">Saved.</p>{/if}
      <input type="hidden" name="id" value={t.id} />
      <label for={`subject-${t.id}`}>Subject</label>
      <input id={`subject-${t.id}`} name="subject" type="text" value={t.subject} required />
      <label for={`body-${t.id}`}>Message</label>
      <textarea id={`body-${t.id}`} name="body_tera" required>{t.body_tera}</textarea>
      <fieldset>
        <legend>Channels</legend>
        {#each CHANNELS as c (c.id)}
          <label>
            <input type="checkbox" name="channels" value={c.id} checked={t.channels.includes(c.id)} />
            {c.label}
          </label>
        {/each}
      </fieldset>
      <label><input type="checkbox" name="active" checked={t.active} /> Active</label>
      <p><button class="button" type="submit">Save</button></p>
    </fieldset>
  </form>
{:else}
  <p>No templates yet. The reminder template is created the first time reminders run.</p>
{/each}
